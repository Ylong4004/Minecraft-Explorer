//! 仅使用原生文件通知，不轮询文件字节、不写入游戏数据。
use crate::platform::{checked_path, path_key};
use anyhow::Result;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TrySendError},
    },
    time::{Duration, Instant},
};

const MAX_QUEUED_EVENTS: usize = 1_024;
const MAX_PENDING_PATHS: usize = 8_192;

#[derive(Clone, Debug, Default)]
pub struct WatchBatch {
    pub paths: Vec<PathBuf>,
    /// 原生队列溢出、监控错误或有界队列丢弃事件。
    /// 调用方必须重新发现目录，不能假设事件路径完整。
    pub overflow: bool,
}

pub struct Monitor {
    watcher: RecommendedWatcher,
    receiver: Receiver<notify::Result<Event>>,
    dropped: Arc<AtomicBool>,
    roots: Vec<PathBuf>,
    root_tokens: BTreeMap<String, u128>,
    pending: BTreeMap<String, PathBuf>,
    overflow: bool,
    first_event: Option<Instant>,
    last_event: Option<Instant>,
    debounce: Duration,
    retry_needed: bool,
    subscriptions_stale: bool,
    root_probe_at: Instant,
}

impl Monitor {
    pub fn new(debounce: Duration) -> Result<Self> {
        let (watcher, receiver, dropped) = backend()?;
        Ok(Self {
            watcher,
            receiver,
            dropped,
            roots: Vec::new(),
            root_tokens: BTreeMap::new(),
            pending: BTreeMap::new(),
            overflow: false,
            first_event: None,
            last_event: None,
            debounce,
            retry_needed: false,
            subscriptions_stale: false,
            root_probe_at: Instant::now(),
        })
    }

    /// 替换全部监控订阅，包含已配置的根或自定义目录及发现的外部
    /// 游戏目录；已被根目录覆盖的嵌套目录去重。
    /// 分别报告无效或重解析路径，不影响其他根目录。
    pub fn reconfigure(&mut self, paths: &[PathBuf]) -> Result<Vec<String>> {
        let previously_retrying = self.retry_needed;
        let mut warnings = Vec::new();
        let mut unique = BTreeMap::new();
        for path in paths {
            match checked_path(path) {
                Ok(checked) if checked.is_dir() => {
                    unique.insert(path_key(&checked), checked);
                }
                Ok(_) => warnings.push(format!("监控目录不是文件夹：{}", path.display())),
                Err(error) => warnings.push(format!("无法监控 {}：{error}", path.display())),
            }
        }
        let mut roots: Vec<PathBuf> = Vec::new();
        for path in unique.into_values() {
            if !roots.iter().any(|root| contains(root, &path)) {
                roots.push(path);
            }
        }
        self.retry_needed = !warnings.is_empty();
        if roots == self.roots && !self.subscriptions_stale {
            // 常规重试或保存设置不能丢弃已排队的变化事件。
            return Ok(warnings);
        }
        let replacing_active = !self.roots.is_empty() || previously_retrying;
        let (mut watcher, receiver, dropped) = backend()?;
        let mut watched = Vec::new();
        for root in roots {
            match watcher.watch(&root, RecursiveMode::Recursive) {
                Ok(()) => watched.push(root),
                Err(error) => warnings.push(format!("无法监控 {}：{error}", root.display())),
            }
        }
        self.watcher = watcher; // 释放时关闭旧的原生订阅。
        self.receiver = receiver;
        self.dropped = dropped;
        self.roots = watched;
        self.root_tokens = self
            .roots
            .iter()
            .filter_map(|root| root_token(root).map(|token| (path_key(root), token)))
            .collect();
        self.pending.clear();
        // 替换订阅无法原子转移旧事件队列。
        self.overflow = replacing_active && !self.roots.is_empty();
        self.first_event = None;
        self.last_event = None;
        if self.overflow {
            self.touch(Instant::now());
        }
        self.retry_needed = !warnings.is_empty();
        self.subscriptions_stale = false;
        self.root_probe_at = Instant::now();
        Ok(warnings)
    }

    pub fn watched_paths(&self) -> &[PathBuf] {
        &self.roots
    }

    /// 低频重试目标路径，在离线根目录重新上线后恢复订阅。
    pub fn needs_retry(&self) -> bool {
        self.retry_needed
    }

    /// 非阻塞排空队列，每次有预算；用静默期合并写事件，
    /// 游戏持续写入时也在最长三秒后刷新。
    pub fn poll(&mut self) -> Option<WatchBatch> {
        let now = Instant::now();
        // 目录路径删除重建后，原生订阅可能仍附着在旧目录对象上；
        // 只检查根目录身份，不探测整个文件树。
        if !self.retry_needed && now.duration_since(self.root_probe_at) >= Duration::from_secs(5) {
            self.root_probe_at = now;
            if self
                .roots
                .iter()
                .any(|root| root_token(root) != self.root_tokens.get(&path_key(root)).copied())
            {
                self.retry_needed = true;
                self.subscriptions_stale = true;
                self.overflow = true;
                self.touch(now);
            }
        }
        if self.dropped.swap(false, Ordering::Relaxed) {
            self.overflow = true;
            self.touch(now);
        }
        for _ in 0..MAX_QUEUED_EVENTS {
            let Ok(result) = self.receiver.try_recv() else {
                break;
            };
            let event = match result {
                Ok(event) => event,
                Err(_) => {
                    self.retry_needed = true;
                    self.subscriptions_stale = true;
                    self.overflow = true;
                    self.touch(now);
                    continue;
                }
            };
            if matches!(event.kind, EventKind::Access(_)) {
                continue;
            }
            if event.need_rescan() {
                self.overflow = true;
                self.touch(now);
            }
            for path in event.paths {
                if !self.roots.iter().any(|root| contains(root, &path)) {
                    continue;
                }
                if matches!(event.kind, EventKind::Remove(_))
                    && self
                        .roots
                        .iter()
                        .any(|root| path_key(root) == path_key(&path))
                {
                    self.retry_needed = true;
                    self.subscriptions_stale = true;
                    self.overflow = true;
                }
                // 已删除路径无法规范化，因此验证最近的现存
                // 祖先路径，同时拒绝新加入的目录联接或链接下的事件。
                if !safe_event_path(&path) {
                    continue;
                }
                if self.pending.len() >= MAX_PENDING_PATHS {
                    self.overflow = true;
                } else {
                    self.pending.insert(path_key(&path), path);
                }
                self.touch(now);
            }
        }
        let first = self.first_event?;
        let last = self.last_event.unwrap_or(first);
        if now.duration_since(last) < self.debounce
            && now.duration_since(first) < Duration::from_secs(3)
        {
            return None;
        }
        let batch = WatchBatch {
            paths: std::mem::take(&mut self.pending).into_values().collect(),
            overflow: self.overflow,
        };
        self.overflow = false;
        self.first_event = None;
        self.last_event = None;
        Some(batch)
    }

    fn touch(&mut self, now: Instant) {
        self.first_event.get_or_insert(now);
        self.last_event = Some(now);
    }
}

fn backend() -> Result<(
    RecommendedWatcher,
    Receiver<notify::Result<Event>>,
    Arc<AtomicBool>,
)> {
    let (sender, receiver) = mpsc::sync_channel(MAX_QUEUED_EVENTS);
    let dropped = Arc::new(AtomicBool::new(false));
    let callback_dropped = dropped.clone();
    let watcher = RecommendedWatcher::new(
        move |event| {
            if matches!(sender.try_send(event), Err(TrySendError::Full(_))) {
                callback_dropped.store(true, Ordering::Relaxed);
            }
        },
        Config::default().with_follow_symlinks(false),
    )?;
    Ok((watcher, receiver, dropped))
}

fn contains(root: &Path, path: &Path) -> bool {
    let root = path_key(root);
    let root = root.trim_end_matches(['/', '\\']);
    let path = path_key(path);
    let suffix = path.strip_prefix(root);
    matches!(suffix, Some("")) || suffix.is_some_and(|suffix| suffix.starts_with(['/', '\\']))
}

fn safe_event_path(path: &Path) -> bool {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(_) => return checked_path(ancestor).is_ok(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return false,
        }
    }
    false
}

fn root_token(path: &Path) -> Option<u128> {
    let checked = checked_path(path).ok()?;
    let metadata = fs::symlink_metadata(&checked).ok()?;
    if !metadata.is_dir() {
        return None;
    }
    #[cfg(windows)]
    {
        use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, GetFileInformationByHandle,
        };
        let directory = fs::OpenOptions::new()
            .read(true)
            .access_mode(0)
            .share_mode(7)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(&checked)
            .ok()?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // 目录句柄在原生查询期间保持有效，之后自动关闭。
        if unsafe { GetFileInformationByHandle(directory.as_raw_handle(), &mut info) } == 0 {
            return None;
        }
        Some(
            ((info.dwVolumeSerialNumber as u128) << 64)
                | ((info.nFileIndexHigh as u128) << 32)
                | info.nFileIndexLow as u128,
        )
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(((metadata.dev() as u128) << 64) | metadata.ino() as u128)
    }
    #[cfg(not(any(windows, unix)))]
    {
        metadata
            .created()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|time| time.as_nanos())
    }
}
