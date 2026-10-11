//! Native file notifications only. No polling of file bytes and no writes to game data.
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
    /// Native queue overflow, watcher error, or our bounded queue dropped an event.
    /// Caller must refresh discovery rather than assume the paths are complete.
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

    /// Replace all watches. Include configured roots/custom paths and discovered external
    /// game directories; nested directories already covered by a root are deduplicated.
    /// Invalid/reparse paths are reported individually without disabling other roots.
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
            // A routine retry/settings save must not discard already queued changes.
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
        self.watcher = watcher; // Drop closes the previous native subscriptions.
        self.receiver = receiver;
        self.dropped = dropped;
        self.roots = watched;
        self.root_tokens = self
            .roots
            .iter()
            .filter_map(|root| root_token(root).map(|token| (path_key(root), token)))
            .collect();
        self.pending.clear();
        // Subscription replacement cannot atomically transfer the old event queue.
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

    /// Retry desired paths at a low frequency when an offline root becomes available.
    pub fn needs_retry(&self) -> bool {
        self.retry_needed
    }

    /// Nonblocking drain with a bounded per-call budget. A quiet period coalesces writes;
    /// a three-second maximum latency also flushes while the game is continuously writing.
    pub fn poll(&mut self) -> Option<WatchBatch> {
        let now = Instant::now();
        // A native subscription can remain attached to an old directory object after its
        // pathname is removed/recreated. Probe only root identities, never the file tree.
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
                // Deleted paths cannot be canonicalized, so validate their nearest existing
                // ancestor. This also rejects events under newly inserted junctions/links.
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
        // The directory handle stays alive for the native query and is then closed by Drop.
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
