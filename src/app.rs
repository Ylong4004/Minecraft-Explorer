use crate::{AppWindow, EntryRow, LauncherRow, SourceRow};
use anyhow::{Context, Result, bail};
use minecraft_explorer::{
    content, duplicates,
    index::Index,
    launcher,
    models::*,
    platform::*,
    scanner::{self, ScanEvent},
    watch,
};
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

enum Command {
    Query(Query, u64),
    Scan,
    ScanPaths(Vec<PathBuf>),
    Annotate(String, bool, Vec<String>),
    Watch(bool),
    Duplicates,
    AddSource(SourceKind, Option<PathBuf>),
    RemoveSource(String),
    SaveLauncher(LauncherInstallation),
    RemoveLauncher(String),
    Map(String, String, String, PathBuf),
    ClearMap(String),
    DefaultLauncher(String),
    Open(String, String),
    Reveal(String),
    Copy(String),
    Detail(String),
    Browse(i32),
}

enum Event {
    Settings(Settings),
    View {
        request: u64,
        items: Vec<Item>,
        games: Vec<Item>,
        versions: Vec<Item>,
        stats: Stats,
    },
    Status(String),
    Progress(f32, String),
    Scanning(bool),
    Dirty,
    Warning(String),
    Detail(Box<Item>, Option<(Vec<u8>, u32, u32)>),
    Browse(i32, PathBuf),
    EditorSaved,
    SourcesChanged,
    Duplicates(Vec<Item>, String),
    RowIcon(String, String, Option<(Vec<u8>, u32, u32)>),
}

#[derive(Default)]
struct UiState {
    settings: Settings,
    items: Vec<Item>,
    games: Vec<Item>,
    versions: Vec<Item>,
    selected: Option<Item>,
    filters: Vec<Option<String>>,
    root_paths: Vec<PathBuf>,
    diagnostics: Vec<String>,
    request: u64,
    query_due: Option<Instant>,
    last_query: Option<Instant>,
    duplicate_items: Vec<Item>,
    row_model: Rc<VecModel<EntryRow>>,
    row_positions: HashMap<String, usize>,
}

#[derive(Default)]
struct IconCache {
    loaded: HashMap<String, Option<slint::Image>>,
    order: VecDeque<String>,
    pending: HashSet<String>,
    deferred: VecDeque<IconRequest>,
    sources: HashMap<String, IconRequest>,
}

#[derive(Clone)]
struct IconRequest {
    id: String,
    key: String,
    path: PathBuf,
    kind: ItemKind,
    icon: Option<PathBuf>,
}

struct TaskState {
    running: Arc<AtomicBool>,
    events: mpsc::Sender<Event>,
}

impl Drop for TaskState {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        let _ = self.events.send(Event::Scanning(false));
        let _ = self.events.send(Event::Dirty);
    }
}

fn icon_key(item: &Item) -> String {
    format!(
        "{}:{:?}:{:?}:{:?}:{:?}:{:?}",
        item.id,
        item.modified_ms,
        item.size,
        item.preview_modified_ms,
        item.icon,
        item.content.as_ref().and_then(|m| m.preview_entry.as_ref())
    )
}

fn icon_worker(requests: mpsc::Receiver<IconRequest>, events: mpsc::Sender<Event>) {
    while let Ok(request) = requests.recv() {
        let pixels = read_icon(&request.path, &request.kind, request.icon.as_deref());
        if events
            .send(Event::RowIcon(request.id, request.key, pixels))
            .is_err()
        {
            break;
        }
    }
}

fn storage_path() -> Result<PathBuf> {
    let base = std::env::var_os("MINECRAFT_EXPLORER_DATA")
        .or_else(|| std::env::var_os("LOCALAPPDATA"))
        .context("缺少 LocalAppData；可设置 MINECRAFT_EXPLORER_DATA 指定缓存目录")?;
    let path = if std::env::var_os("MINECRAFT_EXPLORER_DATA").is_some() {
        PathBuf::from(base)
    } else {
        PathBuf::from(base).join("MinecraftExplorer")
    };
    std::fs::create_dir_all(&path)?;
    Ok(path.join("index.db"))
}

fn decode_icon(item: &Item) -> Option<(Vec<u8>, u32, u32)> {
    read_icon(&item.path, &item.kind, item.icon.as_deref())
}

fn read_icon(
    path: &std::path::Path,
    kind: &ItemKind,
    icon: Option<&std::path::Path>,
) -> Option<(Vec<u8>, u32, u32)> {
    let bytes = icon
        .and_then(|p| read_bounded(p, 2 * 1024 * 1024).ok())
        .or_else(|| content::preview(path, kind))?;
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(1024);
    limits.max_image_height = Some(1024);
    limits.max_alloc = Some(8 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().ok()?.thumbnail(96, 96).into_rgba8();
    let (width, height) = image.dimensions();
    Some((image.into_raw(), width, height))
}

fn resolve_mapping<'a>(
    settings: &'a Settings,
    item: &Item,
    launcher_id: &str,
) -> Option<&'a LaunchMapping> {
    let game_id = item
        .game_dir
        .as_ref()
        .map(|p| format!("instance:{}", path_key(p)));
    settings
        .mappings
        .iter()
        .find(|m| m.subject_id == item.id && m.launcher_id == launcher_id)
        .or_else(|| {
            settings
                .mappings
                .iter()
                .find(|m| Some(&m.subject_id) == game_id.as_ref() && m.launcher_id == launcher_id)
        })
        .or_else(|| {
            item.game_dir.as_ref().and_then(|dir| {
                settings.mappings.iter().find(|m| {
                    m.subject_id == format!("version:{}", path_key(dir))
                        && m.launcher_id == launcher_id
                })
            })
        })
}

fn worker(
    db: PathBuf,
    commands: mpsc::Receiver<Command>,
    events: mpsc::Sender<Event>,
    cancel: Arc<AtomicBool>,
) {
    let result = (|| -> Result<()> {
        let index = Index::open(&db)?;
        let mut settings = index.load_settings()?;
        let scanning = Arc::new(AtomicBool::new(false));
        let mut children = Vec::<std::process::Child>::new();
        let _ = events.send(Event::Settings(settings.clone()));
        let mut monitor = match watch::Monitor::new(Duration::from_millis(900)) {
            Ok(m) => Some(m),
            Err(e) => {
                let _ = events.send(Event::Warning(format!("监控不可用，仍可手动刷新：{e}")));
                None
            }
        };
        let mut watched_paths = Vec::<String>::new();
        let mut pending_paths = Vec::<PathBuf>::new();
        let mut pending_full = false;
        let mut watch_retry = Instant::now();
        let mut watch_incomplete = false;
        loop {
            let monitoring = (|| -> Result<()> {
                if let Some(monitor) = &mut monitor {
                    let mut paths = if settings.watch_enabled {
                        settings
                            .sources
                            .iter()
                            .map(|s| s.path.clone())
                            .chain(
                                index
                                    .query(&Query {
                                        kind: Some(ItemKind::Instance),
                                        ..Default::default()
                                    })?
                                    .into_iter()
                                    .filter_map(|i| i.game_dir),
                            )
                            .collect::<Vec<_>>()
                    } else {
                        Vec::new()
                    };
                    paths.sort_by_key(|p| path_key(p));
                    paths.dedup_by(|a, b| path_key(a) == path_key(b));
                    let keys = paths.iter().map(|p| path_key(p)).collect::<Vec<_>>();
                    watch_incomplete |= monitor.needs_retry();
                    if keys != watched_paths
                        || (watch_incomplete && watch_retry.elapsed() >= Duration::from_secs(15))
                    {
                        let warnings = monitor.reconfigure(&paths)?;
                        watch_incomplete = !warnings.is_empty();
                        watch_retry = Instant::now();
                        for warning in warnings {
                            let _ = events.send(Event::Warning(warning));
                        }
                        watched_paths = keys;
                    }
                    if let Some(batch) = monitor.poll() {
                        pending_full |= batch.overflow
                            || batch.paths.iter().any(|p| {
                                p.file_name().is_some_and(|n| {
                                    n.eq_ignore_ascii_case("launcher_profiles.json")
                                }) || (p
                                    .extension()
                                    .is_some_and(|e| e.eq_ignore_ascii_case("json"))
                                    && p.parent()
                                        .and_then(|p| p.parent())
                                        .and_then(|p| p.file_name())
                                        .is_some_and(|n| n.eq_ignore_ascii_case("versions")))
                            });
                        pending_paths.extend(batch.paths);
                    }
                }
                Ok(())
            })();
            if let Err(e) = monitoring {
                let _ = events.send(Event::Warning(format!("目录监控已暂停，可手动刷新：{e:#}")));
                monitor = None;
            }
            let command = if !scanning.load(Ordering::SeqCst) && pending_full {
                pending_full = false;
                pending_paths.clear();
                Command::Scan
            } else if !scanning.load(Ordering::SeqCst) && !pending_paths.is_empty() {
                Command::ScanPaths(std::mem::take(&mut pending_paths))
            } else {
                match commands.recv_timeout(Duration::from_millis(200)) {
                    Ok(command) => command,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            };
            children.retain_mut(|child| matches!(child.try_wait(), Ok(None)));
            let result = (|| -> Result<()> {
                match command {
                    Command::Query(query, request) => {
                        let items = index.query(&query)?;
                        let games = index.query(&Query {
                            kind: Some(ItemKind::Instance),
                            ..Default::default()
                        })?;
                        let versions = index.query(&Query {
                            kind: Some(ItemKind::Version),
                            ..Default::default()
                        })?;
                        let _ = events.send(Event::View {
                            request,
                            items,
                            games,
                            versions,
                            stats: index.stats()?,
                        });
                    }
                    Command::Scan | Command::ScanPaths(_) => {
                        let paths = if let Command::ScanPaths(paths) = command {
                            Some(paths)
                        } else {
                            None
                        };
                        let full = paths.is_none();
                        let cached = if paths.is_some() {
                            index.query(&Query::default())?
                        } else {
                            Vec::new()
                        };
                        if scanning.swap(true, Ordering::SeqCst) {
                            return Ok(());
                        }
                        cancel.store(false, Ordering::SeqCst);
                        let _ = events.send(Event::Scanning(true));
                        let _ = events.send(Event::Duplicates(
                            Vec::new(),
                            "索引正在刷新；完成后可重新检测重复候选".into(),
                        ));
                        let _ = events.send(Event::Status("正在发现目录…".into()));
                        let scan_settings = settings.clone();
                        let scan_db = db.clone();
                        let output = events.clone();
                        let scan_cancel = cancel.clone();
                        let task = TaskState {
                            running: scanning.clone(),
                            events: events.clone(),
                        };
                        std::thread::Builder::new()
                            .name("scan".into())
                            .spawn(move || {
                                let _task = task;
                                let result = (|| -> Result<bool> {
                                    let mut db = Index::open(&scan_db)?;
                                    let generation = now_ms();
                                    let mut last_update = Instant::now() - Duration::from_secs(1);
                                    let mut write_error = None;
                                    let mut handle = |event| {
                                        if write_error.is_some() {
                                            return;
                                        }
                                        let result = match event {
                                            ScanEvent::Discovered(items) => {
                                                db.discovered(&items, generation)
                                            }
                                            ScanEvent::Updated(item) => {
                                                db.upsert(&item, generation)
                                            }
                                            ScanEvent::Warning(warning) => {
                                                let _ = output.send(Event::Warning(warning));
                                                Ok(())
                                            }
                                            ScanEvent::Progress {
                                                done,
                                                total,
                                                message,
                                            } => {
                                                if last_update.elapsed()
                                                    >= Duration::from_millis(200)
                                                {
                                                    let _ = output.send(Event::Progress(
                                                        done as f32 / total.max(1) as f32,
                                                        message,
                                                    ));
                                                    let _ = output.send(Event::Dirty);
                                                    last_update = Instant::now();
                                                }
                                                Ok(())
                                            }
                                        };
                                        if let Err(error) = result {
                                            write_error = Some(error);
                                            scan_cancel.store(true, Ordering::SeqCst);
                                        }
                                    };
                                    let mut removed = Vec::new();
                                    let mut invalidated = Vec::new();
                                    let report = if let Some(paths) = paths {
                                        let refresh = scanner::refresh_changed(
                                            &scan_settings,
                                            &cached,
                                            &paths,
                                            &scan_cancel,
                                            &mut handle,
                                        );
                                        removed = refresh.removed_ids;
                                        invalidated = refresh.invalidated;
                                        refresh.scan
                                    } else {
                                        scanner::scan(&scan_settings, &scan_cancel, &mut handle)
                                    };
                                    if let Some(error) = write_error {
                                        return Err(error);
                                    }
                                    if !report.cancelled {
                                        if full {
                                            for source in &report.completed_sources {
                                                db.finish_source(source, generation)?;
                                            }
                                        } else {
                                            db.remove_items(&removed)?;
                                            for item in invalidated {
                                                db.upsert(&item, generation)?;
                                            }
                                        }
                                        for source in &report.offline_sources {
                                            db.mark_unavailable(source)?;
                                        }
                                    }
                                    let status = if report.cancelled {
                                        "扫描已取消；保留已读取结果，可刷新重试".to_string()
                                    } else {
                                        format!(
                                            "扫描完成：{} 个条目；{} 条诊断，逻辑大小（跳过链接）",
                                            report.items.len(),
                                            report.warnings.len()
                                        )
                                    };
                                    let _ = output.send(Event::Status(status));
                                    Ok(report.cancelled)
                                })();
                                if let Err(error) = result {
                                    let _ =
                                        output.send(Event::Status(format!("扫描失败：{error:#}")));
                                }
                            })
                            .context("无法创建扫描线程")?;
                    }
                    Command::Annotate(id, favorite, tags) => {
                        index.annotate(&id, favorite, &tags)?;
                        let _ = events.send(Event::Dirty);
                        let _ = events
                            .send(Event::Status("收藏与标签已保存；只写入本程序数据库".into()));
                    }
                    Command::Watch(enabled) => {
                        settings.watch_enabled = enabled;
                        index.save_settings(&settings)?;
                        if !enabled {
                            pending_paths.clear();
                            pending_full = false;
                        }
                        let _ = events.send(Event::Settings(settings.clone()));
                        let _ = events.send(Event::Status(
                            if enabled {
                                "目录监控已开启；变化合并后增量刷新"
                            } else {
                                "目录监控已关闭，可使用 F5 手动刷新"
                            }
                            .into(),
                        ));
                    }
                    Command::Duplicates => {
                        let items = index.query(&Query::default())?;
                        if scanning.swap(true, Ordering::SeqCst) {
                            bail!("请等待或取消当前扫描任务");
                        }
                        cancel.store(false, Ordering::SeqCst);
                        let _ = events.send(Event::Scanning(true));
                        let output = events.clone();
                        let flag = cancel.clone();
                        let task = TaskState {
                            running: scanning.clone(),
                            events: events.clone(),
                        };
                        std::thread::Builder::new().name("duplicates".into()).spawn(move || {
                            let _task = task;
                            let report=duplicates::find(&items,&flag,|p|{let _ = output.send(Event::Progress(if p.total==0 {0.}else {p.done as f32/p.total as f32},format!("只读重复候选检测：{} / {} 个文件",p.done,p.total)));});
                            let mut matches=Vec::new();
                            for (n,group) in report.groups.iter().enumerate() {for item in &group.items {let mut item=item.clone();item.ownership=format!("候选组 {} · {}",n+1,item.ownership);item.metadata_status.push_str(&format!("\nBLAKE3：{}；仅在检测完成时字节一致，不表示可安全删除",group.hash));matches.push(item);}}
                            for warning in report.warnings {let _ = output.send(Event::Warning(warning));}
                            let text=format!("{} 个重复候选组 / {} 个条目；已完整哈希 {} 个文件、{}。{}",report.groups.len(),matches.len(),report.hashed_files,bytes_label(Some(report.hashed_bytes),true),if report.cancelled {"任务已取消，结果不完整"}else {"只读检测；未删除文件。内容变化后请重新检测"});
                            let _ = output.send(Event::Duplicates(matches,text.clone()));
                            let _ = output.send(Event::Status(text));
                        }).context("无法创建重复检测线程")?;
                    }
                    Command::AddSource(kind, associated_root) => {
                        if scanning.load(Ordering::SeqCst) {
                            bail!("请先取消扫描再修改目录配置");
                        }
                        if let Some(path) = rfd::FileDialog::new()
                            .set_title("选择实际的 Minecraft 根目录 / 游戏目录")
                            .pick_folder()
                        {
                            let path = checked_path(&path)?;
                            if settings
                                .sources
                                .iter()
                                .any(|s| path_key(&s.path) == path_key(&path))
                            {
                                bail!("该目录已添加");
                            }
                            settings.sources.push(Source {
                                path,
                                kind,
                                associated_root,
                            });
                            index.save_settings(&settings)?;
                            let _ = events.send(Event::Settings(settings.clone()));
                            let _ = events.send(Event::Status("目录已添加；正在刷新索引".into()));
                            let _ = events.send(Event::SourcesChanged);
                        }
                    }
                    Command::RemoveSource(path) => {
                        if scanning.load(Ordering::SeqCst) {
                            bail!("请先取消扫描再修改目录配置");
                        }
                        let source = settings
                            .sources
                            .iter()
                            .find(|s| display_path(&s.path) == path)
                            .context("目录配置已改变")?
                            .clone();
                        settings
                            .sources
                            .retain(|s| path_key(&s.path) != path_key(&source.path));
                        index.save_settings(&settings)?;
                        index.remove_source(&path_key(&source.path))?;
                        let _ = events.send(Event::Settings(settings.clone()));
                        let _ = events.send(Event::Dirty);
                        let _ = events.send(Event::Status("已移除索引关联，游戏文件未修改".into()));
                        let _ = events.send(Event::SourcesChanged);
                    }
                    Command::SaveLauncher(mut installation) => {
                        installation.executable = checked_path(&installation.executable)?;
                        installation.working_dir = checked_path(&installation.working_dir)?;
                        installation.root = checked_path(&installation.root)?;
                        installation.java =
                            installation.java.map(|p| checked_path(&p)).transpose()?;
                        launcher::validate_installation(&installation)?;
                        installation.checked_ms = now_ms();
                        if installation.id.is_empty() {
                            installation.id = format!("launcher-{}", now_ms());
                        }
                        let mut updated = settings.clone();
                        updated.launchers.retain(|l| l.id != installation.id);
                        updated.launchers.push(installation);
                        index.save_settings(&updated)?;
                        settings = updated;
                        let _ = events.send(Event::Settings(settings.clone()));
                        let _ = events.send(Event::EditorSaved);
                        let _ = events.send(Event::Status(
                            "启动器配置已保存；OpenOnly：直接启动指定版本尚未验证".into(),
                        ));
                    }
                    Command::RemoveLauncher(id) => {
                        settings.launchers.retain(|l| l.id != id);
                        settings.mappings.retain(|m| m.launcher_id != id);
                        settings.instance_defaults.retain(|_, l| l != &id);
                        if settings.default_launcher.as_ref() == Some(&id) {
                            settings.default_launcher = None;
                        }
                        index.save_settings(&settings)?;
                        let _ = events.send(Event::Settings(settings.clone()));
                    }
                    Command::Map(subject_id, launcher_id, version_id, game_dir) => {
                        let subject = index.get(&subject_id)?.context("所选条目已失效")?;
                        let version = index.get(&version_id)?.context("目标版本已失效")?;
                        let installation = settings
                            .launchers
                            .iter()
                            .find(|l| l.id == launcher_id)
                            .context("请选择启动器")?;
                        let mapping =
                            launcher::create_mapping(&subject, installation, &version, &game_dir)?;
                        settings
                            .instance_defaults
                            .insert(subject.id.clone(), launcher_id.clone());
                        if let Some(game_dir) = &subject.game_dir {
                            settings
                                .instance_defaults
                                .insert(format!("instance:{}", path_key(game_dir)), launcher_id);
                        }
                        settings.mappings.retain(|m| {
                            m.subject_id != mapping.subject_id
                                || m.launcher_id != mapping.launcher_id
                        });
                        settings.mappings.push(mapping);
                        index.save_settings(&settings)?;
                        let _ = events.send(Event::Settings(settings.clone()));
                        let _ = events.send(Event::Status(
                            "手动关联已保存；打开启动器后仍需手动选择目标版本".into(),
                        ));
                        let _ = events.send(Event::EditorSaved);
                    }
                    Command::ClearMap(id) => {
                        settings.mappings.retain(|m| m.subject_id != id);
                        settings.instance_defaults.remove(&id);
                        index.save_settings(&settings)?;
                        let _ = events.send(Event::Settings(settings.clone()));
                        let _ = events.send(Event::EditorSaved);
                    }
                    Command::DefaultLauncher(id) => {
                        if !settings.launchers.iter().any(|l| l.id == id) {
                            bail!("请先添加启动器");
                        }
                        settings.default_launcher = Some(id);
                        index.save_settings(&settings)?;
                        let _ = events.send(Event::Settings(settings.clone()));
                        let _ = events.send(Event::Status("已保存全局默认启动器".into()));
                    }
                    Command::Open(subject_id, launcher_id) => {
                        let installation = settings
                            .launchers
                            .iter()
                            .find(|l| l.id == launcher_id)
                            .context("启动器未配置")?;
                        let subject = index.get(&subject_id)?.context("所选条目已失效")?;
                        let mapping = resolve_mapping(&settings, &subject, &launcher_id);
                        if !subject.available {
                            bail!("所选条目的来源目录离线，请连接后刷新");
                        }
                        checked_path(&subject.path)?;
                        if let Some(mapping) = mapping {
                            let version = index
                                .get(&format!("version:{}", path_key(&mapping.version_path)))?
                                .context("目标版本已失效")?;
                            launcher::validate_mapping(mapping, installation, &version)?;
                        } else if path_key(&subject.root) != path_key(&installation.root) {
                            bail!("所选启动器与条目属于不同根目录，请切换启动器或修改关联");
                        }
                        let plan = launcher::adapter(&installation.kind)
                            .open_plan(installation)
                            .context("请检查所选安装，或手动切换同根目录的其他启动器")?;
                        let mut child = launcher::spawn(&plan)
                            .context("打开所选安装失败；可手动切换同根目录的其他启动器后重试")?;
                        std::thread::sleep(Duration::from_millis(250));
                        let message = match child.try_wait()? {
                            Some(exit) if !exit.success() => {
                                format!(
                                    "Failed：启动器立即退出，代码 {exit}；检查 Java 和安装文件，或手动切换同根目录的其他安装"
                                )
                            }
                            Some(_) => "启动器入口已退出；可能交给已有进程，未确认游戏启动".into(),
                            None => "启动器进程已创建；请手动选择目标版本，未确认游戏启动".into(),
                        };
                        children.push(child);
                        if !message.starts_with("Failed") {
                            index.record_opened(&subject_id)?;
                            let _ = events.send(Event::Dirty);
                        }
                        let _ = events.send(Event::Status(message));
                    }
                    Command::Reveal(id) => {
                        let item = index.get(&id)?.context("条目已失效")?;
                        reveal(&item.path)?;
                        index.record_opened(&id)?;
                        let _ = events.send(Event::Dirty);
                        let _ = events.send(Event::Status("已向文件资源管理器发送定位请求".into()));
                    }
                    Command::Copy(text) => {
                        arboard::Clipboard::new()?.set_text(text)?;
                        let _ = events.send(Event::Status("已复制到剪贴板".into()));
                    }
                    Command::Detail(id) => {
                        if let Some(item) = index.get(&id)? {
                            index.record_opened(&id)?;
                            let icon = decode_icon(&item);
                            let _ = events.send(Event::Detail(Box::new(item), icon));
                            let _ = events.send(Event::Dirty);
                        }
                    }
                    Command::Browse(field) => {
                        let dialog = rfd::FileDialog::new();
                        let path = if field == 1 {
                            dialog.pick_folder()
                        } else {
                            dialog.pick_file()
                        };
                        if let Some(path) = path {
                            let _ = events.send(Event::Browse(field, path));
                        }
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                let _ = events.send(Event::Status(format!("操作失败：{error:#}")));
            }
        }
        cancel.store(true, Ordering::SeqCst);
        Ok(())
    })();
    if let Err(error) = result {
        let _ = events.send(Event::Status(format!("索引服务失败：{error:#}")));
    }
}

fn strings(values: Vec<String>) -> ModelRc<SharedString> {
    Rc::new(VecModel::from(
        values
            .into_iter()
            .map(SharedString::from)
            .collect::<Vec<_>>(),
    ))
    .into()
}

fn time_label(time: Option<i64>) -> String {
    time.and_then(chrono::DateTime::from_timestamp_millis)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "未知".into())
}

fn detail(ui: &AppWindow, state: &UiState, item: &Item) {
    ui.set_detail_title(item.name.clone().into());
    let size = bytes_label(item.size, item.size_complete);
    ui.set_detail_text(format!("{}\n\n真实路径\n{}\n\n实际游戏目录\n{}\n\n归属\n{}\n\n版本定义 ID\n{}\n\n世界声明的历史版本\n{}\n\n逻辑大小\n{}\n\n最后游玩\n{}\n修改时间\n{}\n\n信息来源 / 状态\n{}{}", item.kind.label(), display_path(&item.path),
        item.game_dir.as_ref().map(|p| display_path(p)).unwrap_or_else(|| "未确认；请手动关联".into()), item.ownership,
        item.version_id.as_deref().unwrap_or("未知 / 共享"), item.game_version.as_deref().unwrap_or("未知"), size,
        time_label(item.last_played_ms), time_label(item.modified_ms), item.metadata_status,
        if item.available { "" } else { "\n来源离线：当前显示缓存" }).into());
    if let Some(meta) = &item.content {
        let extra = format!(
            "\n\n内容声明信息\nID：{}\n版本：{}\n描述：{}\n作者：{}\nLoader：{}\n兼容性：{}",
            meta.declared_id.as_deref().unwrap_or("未知"),
            meta.declared_version.as_deref().unwrap_or("未知"),
            meta.description.as_deref().unwrap_or("未提供"),
            meta.authors.join("、"),
            meta.loaders.join("、"),
            meta.compatibility.join("、")
        );
        ui.set_detail_text(format!("{}{extra}", ui.get_detail_text()).into());
    }
    launch_info(ui, state);
}

fn selected_launcher<'a>(ui: &AppWindow, state: &'a UiState) -> Option<&'a LauncherInstallation> {
    state
        .settings
        .launchers
        .get(ui.get_launcher_index().max(0) as usize)
}

fn launch_info(ui: &AppWindow, state: &UiState) {
    let Some(item) = &state.selected else {
        return;
    };
    let Some(installation) = selected_launcher(ui, state) else {
        ui.set_launch_status("LauncherNotConfigured".into());
        ui.set_launch_hint("在设置中添加 PCL2 / PCL CE / HMCL 安装文件".into());
        ui.set_can_open(false);
        ui.set_target_name("".into());
        return;
    };
    let mapping = resolve_mapping(&state.settings, item, &installation.id);
    let target = mapping
        .map(|m| m.version_id.clone())
        .or_else(|| item.version_id.clone())
        .unwrap_or_default();
    ui.set_target_name(target.clone().into());
    let same_root = path_key(&item.root) == path_key(&installation.root);
    let valid_mapping = mapping.is_none_or(|m| {
        state
            .versions
            .iter()
            .find(|v| v.path == m.version_path)
            .is_some_and(|v| {
                v.version_id.as_deref() == Some(m.version_id.as_str())
                    && path_key(&v.root) == path_key(&installation.root)
            })
    });
    ui.set_can_open(same_root && valid_mapping && item.available);
    ui.set_launch_status(
        if !same_root || !valid_mapping {
            "Unmapped"
        } else {
            "OpenOnly"
        }
        .into(),
    );
    ui.set_launch_hint(
        if !same_root {
            "所选启动器属于另一个根目录，请切换或重新关联".to_string()
        } else if !valid_mapping {
            "映射目标已经失效，请刷新并重新设置关联".to_string()
        } else if target.is_empty() {
            "无法确定目标版本；共享目录需手动关联。当前只能打开启动器。".to_string()
        } else {
            format!(
                "只能打开 {}。请在启动器内手动选择：{}\n根目录：{}",
                installation.kind.label(),
                target,
                display_path(&installation.root)
            )
        }
        .into(),
    );
}

fn apply_settings(ui: &AppWindow, state: &mut UiState, settings: Settings) {
    let previous = selected_launcher(ui, state).map(|l| l.id.clone());
    state.settings = settings;
    ui.set_watch_enabled(state.settings.watch_enabled);
    ui.set_source_labels(strings(
        std::iter::once("全部来源 / 根目录".into())
            .chain(state.settings.sources.iter().map(|s| display_path(&s.path)))
            .collect(),
    ));
    ui.set_sources(
        Rc::new(VecModel::from(
            state
                .settings
                .sources
                .iter()
                .map(|s| SourceRow {
                    path: display_path(&s.path).into(),
                    label: if s.kind == SourceKind::Root {
                        "Minecraft 根目录"
                    } else {
                        "自定义游戏目录"
                    }
                    .into(),
                })
                .collect::<Vec<_>>(),
        ))
        .into(),
    );
    ui.set_launchers(
        Rc::new(VecModel::from(
            state
                .settings
                .launchers
                .iter()
                .map(|l| LauncherRow {
                    id: l.id.clone().into(),
                    label: format!(
                        "{}{} · OpenOnly",
                        l.kind.label(),
                        if state.settings.default_launcher.as_ref() == Some(&l.id) {
                            "（默认）"
                        } else {
                            ""
                        }
                    )
                    .into(),
                    path: display_path(&l.executable).into(),
                })
                .collect::<Vec<_>>(),
        ))
        .into(),
    );
    ui.set_launcher_labels(strings(
        state
            .settings
            .launchers
            .iter()
            .map(|l| format!("{} · {}", l.kind.label(), display_path(&l.executable)))
            .collect(),
    ));
    state.root_paths = state
        .settings
        .sources
        .iter()
        .filter(|s| s.kind == SourceKind::Root)
        .map(|s| s.path.clone())
        .collect();
    ui.set_root_labels(strings(
        state.root_paths.iter().map(|p| display_path(p)).collect(),
    ));
    let preferred = previous.or_else(|| state.settings.default_launcher.clone());
    if let Some(position) = state
        .settings
        .launchers
        .iter()
        .position(|l| Some(&l.id) == preferred.as_ref())
    {
        ui.set_launcher_index(position as i32);
    }
    launch_info(ui, state);
}

fn queue_query(ui: &AppWindow, state: &mut UiState, sender: &mpsc::Sender<Command>) {
    state.request += 1;
    let kind = match ui.get_page() {
        1 => None,
        2 => Some(ItemKind::World),
        3 => Some(ItemKind::ResourcePack),
        4 => Some(ItemKind::Mod),
        5 => Some(ItemKind::ShaderPack),
        9 => Some(ItemKind::DataPack),
        _ => None,
    };
    let sort = match ui.get_sort_index() {
        1 => Sort::SizeDesc,
        2 => Sort::SizeAsc,
        3 => Sort::TimeDesc,
        4 => Sort::TimeAsc,
        5 => Sort::Ownership,
        6 => Sort::Recent,
        _ => Sort::Name,
    };
    let query = Query {
        text: ui.get_search().to_string(),
        kind,
        game_dir: state
            .filters
            .get(ui.get_filter_index().max(0) as usize)
            .cloned()
            .flatten(),
        sort: if ui.get_page() == 11 {
            Sort::Recent
        } else {
            sort
        },
        source: if ui.get_source_index() > 0 {
            state
                .settings
                .sources
                .get((ui.get_source_index() - 1) as usize)
                .map(|s| path_key(&s.path))
        } else {
            None
        },
        favorite_only: ui.get_favorite_only() || ui.get_page() == 10,
        tag: (!ui.get_tag_filter().trim().is_empty())
            .then(|| ui.get_tag_filter().trim().to_string()),
        recent_only: ui.get_page() == 11,
    };
    let _ = sender.send(Command::Query(query, state.request));
    state.query_due = None;
    state.last_query = Some(Instant::now());
}

pub fn run() -> Result<()> {
    let db = storage_path()?;
    Index::open(&db)?;
    let ui = AppWindow::new()?;
    let (sender, commands) = mpsc::channel();
    let (events, receiver) = mpsc::channel();
    let (icon_sender, icon_requests) = mpsc::sync_channel(64);
    let icon_events = events.clone();
    std::thread::spawn(move || icon_worker(icon_requests, icon_events));
    let icon_cache = Rc::new(RefCell::new(IconCache::default()));
    let request_cache = icon_cache.clone();
    let row_icon_sender = icon_sender.clone();
    ui.on_request_row_icon(move |id, key| {
        if id.is_empty() || key.is_empty() {
            return;
        }
        let mut cache = request_cache.borrow_mut();
        if cache.loaded.contains_key(key.as_str()) || cache.pending.contains(key.as_str()) {
            return;
        }
        let Some(request) = cache
            .sources
            .get(id.as_str())
            .filter(|r| key == r.key)
            .cloned()
        else {
            return;
        };
        cache.pending.insert(key.to_string());
        match row_icon_sender.try_send(request) {
            Ok(()) => (),
            Err(mpsc::TrySendError::Full(request)) => {
                if cache.deferred.len() >= 64
                    && let Some(old) = cache.deferred.pop_front()
                {
                    cache.pending.remove(&old.key);
                }
                cache.deferred.push_back(request);
            }
            Err(mpsc::TrySendError::Disconnected(request)) => {
                cache.pending.remove(&request.key);
                cache.loaded.insert(request.key, None);
            }
        }
    });
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    std::thread::spawn(move || worker(db, commands, events, worker_cancel));
    let state = Rc::new(RefCell::new(UiState::default()));
    state.borrow_mut().diagnostics.push("阶段 2：只读内容索引。PCL2 / PCL CE / HMCL 当前仅具备 OpenLauncher；指定实例直接启动尚未验证。\n大小口径：逻辑字节；内容与世界、游戏目录大小不可叠加。\n监控变化会增量刷新叶条目，实例总大小需 F5 完整刷新；收藏与标签只写入本程序数据库。\n重复检测只列出完整哈希一致的候选，不提供删除。\n不跟随符号链接、junction 或重解析点。".into());
    let weak = ui.as_weak();
    let query_state = state.clone();
    ui.on_query_changed(move || {
        if let Ok(mut state) = query_state.try_borrow_mut() {
            state.query_due = Some(Instant::now() + Duration::from_millis(180));
        }
    });
    let nav_state = state.clone();
    let nav_sender = sender.clone();
    let nav_ui = weak.clone();
    ui.on_navigate(move |_| {
        if let Some(ui) = nav_ui.upgrade() {
            ui.set_selected_id("".into());
            nav_state.borrow_mut().selected = None;
            queue_query(&ui, &mut nav_state.borrow_mut(), &nav_sender);
        }
    });
    let refresh_sender = sender.clone();
    ui.on_refresh(move || {
        let _ = refresh_sender.send(Command::Scan);
    });
    let cancel_flag = cancel.clone();
    let cancel_ui = weak.clone();
    ui.on_cancel_scan(move || {
        cancel_flag.store(true, Ordering::SeqCst);
        if let Some(ui) = cancel_ui.upgrade() {
            ui.set_status("正在取消扫描…".into());
        }
    });
    let add_sender = sender.clone();
    let add_state = state.clone();
    let add_ui = weak.clone();
    ui.on_add_source(move |kind| {
        if let Some(ui) = add_ui.upgrade() {
            let associated_root = if kind == 1 {
                add_state
                    .borrow()
                    .root_paths
                    .get(ui.get_custom_root_index().max(0) as usize)
                    .cloned()
            } else {
                None
            };
            let _ = add_sender.send(Command::AddSource(
                if kind == 0 {
                    SourceKind::Root
                } else {
                    SourceKind::Custom
                },
                associated_root,
            ));
        }
    });
    let remove_sender = sender.clone();
    ui.on_remove_source(move |path| {
        let _ = remove_sender.send(Command::RemoveSource(path.to_string()));
    });
    let select_state = state.clone();
    let select_sender = sender.clone();
    let select_ui = weak.clone();
    ui.on_selected(move |id| {
        if let Some(ui) = select_ui.upgrade() {
            let mut state = select_state.borrow_mut();
            state.selected = state.items.iter().find(|i| i.id == id.as_str()).cloned();
            ui.set_has_icon(false);
            if let Some(item) = &state.selected {
                let preferred = state
                    .settings
                    .instance_defaults
                    .get(&item.id)
                    .cloned()
                    .or_else(|| {
                        item.game_dir.as_ref().and_then(|p| {
                            state
                                .settings
                                .instance_defaults
                                .get(&format!("instance:{}", path_key(p)))
                                .cloned()
                        })
                    })
                    .or_else(|| {
                        item.game_dir.as_ref().and_then(|p| {
                            state
                                .settings
                                .instance_defaults
                                .get(&format!("version:{}", path_key(p)))
                                .cloned()
                        })
                    })
                    .or_else(|| state.settings.default_launcher.clone());
                if let Some(index) = state
                    .settings
                    .launchers
                    .iter()
                    .position(|l| Some(&l.id) == preferred.as_ref())
                {
                    ui.set_launcher_index(index as i32);
                }
                detail(&ui, &state, item);
                ui.set_detail_favorite(item.favorite);
                ui.set_detail_tags(item.tags.join(", ").into());
            }
            let _ = select_sender.send(Command::Detail(id.to_string()));
        }
    });
    let reveal_sender = sender.clone();
    let reveal_ui = weak.clone();
    ui.on_reveal_selected(move || {
        if let Some(ui) = reveal_ui.upgrade() {
            let _ = reveal_sender.send(Command::Reveal(ui.get_selected_id().to_string()));
        }
    });
    let copy_sender = sender.clone();
    let copy_state = state.clone();
    ui.on_copy_selected(move || {
        if let Some(item) = &copy_state.borrow().selected {
            let _ = copy_sender.send(Command::Copy(display_path(&item.path)));
        }
    });
    let target_sender = sender.clone();
    let target_ui = weak.clone();
    ui.on_copy_target(move || {
        if let Some(ui) = target_ui.upgrade() {
            let _ = target_sender.send(Command::Copy(ui.get_target_name().to_string()));
        }
    });
    let open_sender = sender.clone();
    let open_state = state.clone();
    let open_ui = weak.clone();
    ui.on_open_launcher(move || {
        if let Some(ui) = open_ui.upgrade()
            && let Some(l) = selected_launcher(&ui, &open_state.borrow())
        {
            let _ = open_sender.send(Command::Open(
                ui.get_selected_id().to_string(),
                l.id.clone(),
            ));
        }
    });
    let launch_state = state.clone();
    let launch_ui = weak.clone();
    ui.on_launcher_changed(move || {
        if let Some(ui) = launch_ui.upgrade()
            && let Ok(state) = launch_state.try_borrow()
        {
            launch_info(&ui, &state);
        }
    });
    let map_ui = weak.clone();
    let begin_map_state = state.clone();
    ui.on_begin_mapping(move || {
        if let Some(ui) = map_ui.upgrade() {
            let state = begin_map_state.borrow();
            if let Some(item) = &state.selected {
                if let Some(position) = state.games.iter().position(|g| g.game_dir == item.game_dir)
                {
                    ui.set_mapping_game_index(position as i32);
                }
                if let Some(position) = state.versions.iter().position(|v| {
                    v.id == item.id
                        || (v.version_id.is_some()
                            && v.version_id == item.version_id
                            && v.root == item.root)
                }) {
                    ui.set_version_index(position as i32);
                }
            }
            ui.set_mapping_visible(true);
        }
    });
    let map_state = state.clone();
    let map_ui = weak.clone();
    let map_sender = sender.clone();
    ui.on_save_mapping(move || {
        if let Some(ui) = map_ui.upgrade() {
            let state = map_state.borrow();
            if let (Some(l), Some(v), Some(g)) = (
                selected_launcher(&ui, &state),
                state.versions.get(ui.get_version_index().max(0) as usize),
                state.games.get(ui.get_mapping_game_index().max(0) as usize),
            ) {
                let _ = map_sender.send(Command::Map(
                    ui.get_selected_id().to_string(),
                    l.id.clone(),
                    v.id.clone(),
                    g.path.clone(),
                ));
            } else {
                ui.set_status("请先添加启动器并扫描可用版本定义".into());
            }
        }
    });
    let clear_sender = sender.clone();
    let clear_ui = weak.clone();
    ui.on_clear_mapping(move || {
        if let Some(ui) = clear_ui.upgrade() {
            let _ = clear_sender.send(Command::ClearMap(ui.get_selected_id().to_string()));
        }
    });
    let edit_state = state.clone();
    let edit_ui = weak.clone();
    ui.on_edit_launcher(move |id| {
        if let Some(ui) = edit_ui.upgrade() {
            let state = edit_state.borrow();
            let installation = state
                .settings
                .launchers
                .iter()
                .find(|l| l.id == id.as_str());
            ui.set_edit_id(id);
            ui.set_edit_kind(
                installation
                    .map(|l| match l.kind {
                        LauncherKind::Pcl2 => 0,
                        LauncherKind::Hmcl => 1,
                        LauncherKind::PclCe => 2,
                    })
                    .unwrap_or(0),
            );
            ui.set_edit_executable(
                installation
                    .map(|l| display_path(&l.executable))
                    .unwrap_or_default()
                    .into(),
            );
            ui.set_edit_workdir(
                installation
                    .map(|l| display_path(&l.working_dir))
                    .unwrap_or_default()
                    .into(),
            );
            ui.set_edit_java(
                installation
                    .and_then(|l| l.java.as_ref())
                    .map(|p| display_path(p))
                    .unwrap_or_default()
                    .into(),
            );
            ui.set_edit_root_index(
                installation
                    .and_then(|l| state.root_paths.iter().position(|p| p == &l.root))
                    .unwrap_or(0) as i32,
            );
            ui.set_editor_visible(true);
        }
    });
    let save_state = state.clone();
    let save_ui = weak.clone();
    let save_sender = sender.clone();
    ui.on_save_launcher(move || {
        if let Some(ui) = save_ui.upgrade() {
            let state = save_state.borrow();
            let Some(root) = state
                .root_paths
                .get(ui.get_edit_root_index().max(0) as usize)
            else {
                ui.set_status("请先添加 Minecraft 根目录".into());
                return;
            };
            let java = ui.get_edit_java().to_string();
            let installation = LauncherInstallation {
                id: ui.get_edit_id().to_string(),
                kind: match ui.get_edit_kind() {
                    0 => LauncherKind::Pcl2,
                    2 => LauncherKind::PclCe,
                    _ => LauncherKind::Hmcl,
                },
                executable: PathBuf::from(ui.get_edit_executable().as_str()),
                working_dir: PathBuf::from(ui.get_edit_workdir().as_str()),
                root: root.clone(),
                java: (!java.trim().is_empty()).then(|| PathBuf::from(java)),
                checked_ms: 0,
            };
            let _ = save_sender.send(Command::SaveLauncher(installation));
        }
    });
    let delete_sender = sender.clone();
    ui.on_remove_launcher(move |id| {
        let _ = delete_sender.send(Command::RemoveLauncher(id.to_string()));
    });
    let browse_sender = sender.clone();
    ui.on_browse_editor(move |field| {
        let _ = browse_sender.send(Command::Browse(field));
    });
    let default_sender = sender.clone();
    let default_state = state.clone();
    let default_ui = weak.clone();
    ui.on_set_global_default(move || {
        if let Some(ui) = default_ui.upgrade()
            && let Some(l) = selected_launcher(&ui, &default_state.borrow())
        {
            let _ = default_sender.send(Command::DefaultLauncher(l.id.clone()));
        }
    });
    let annotation_ui = weak.clone();
    let annotation_sender = sender.clone();
    ui.on_annotate_selected(move || {
        if let Some(ui) = annotation_ui.upgrade() {
            let tags = ui
                .get_detail_tags()
                .split([',', '，'])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            let _ = annotation_sender.send(Command::Annotate(
                ui.get_selected_id().to_string(),
                ui.get_detail_favorite(),
                tags,
            ));
        }
    });
    let watch_ui = weak.clone();
    let watch_sender = sender.clone();
    ui.on_watch_changed(move || {
        if let Some(ui) = watch_ui.upgrade() {
            let _ = watch_sender.send(Command::Watch(ui.get_watch_enabled()));
        }
    });
    let duplicate_sender = sender.clone();
    ui.on_find_duplicates(move || {
        let _ = duplicate_sender.send(Command::Duplicates);
    });

    let poll_state = state.clone();
    let poll_ui = weak.clone();
    let poll_sender = sender.clone();
    let poll_icons = icon_cache.clone();
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(60), move || {
        let Some(ui) = poll_ui.upgrade() else { return; }; let mut state = poll_state.borrow_mut();
        for event in receiver.try_iter().take(100) {
            match event {
                Event::Settings(settings) => {
                    let first = state.last_query.is_none();
                    let previous_source=ui.get_source_index().checked_sub(1).and_then(|i|state.settings.sources.get(i as usize)).map(|s|path_key(&s.path));
                    apply_settings(&ui,&mut state,settings);
                    ui.set_source_index(previous_source.and_then(|key|state.settings.sources.iter().position(|s|path_key(&s.path)==key)).map(|i|i as i32+1).unwrap_or(0));
                    if first { queue_query(&ui,&mut state,&poll_sender); let _ = poll_sender.send(Command::Scan); }
                    else {state.query_due=Some(Instant::now());}
                }
                Event::View { request, mut items, games, versions, stats } => {
                    if request != state.request { continue; }
                    if ui.get_page() == 1 { items.retain(|i| matches!(i.kind,ItemKind::Version|ItemKind::Instance)); }
                    if ui.get_page() == 12 {
                        let candidates: HashMap<_,_>=state.duplicate_items.iter().map(|i|(&i.id,i)).collect();
                        items=items.into_iter().filter_map(|mut item|candidates.get(&item.id).map(|candidate| {item.ownership=candidate.ownership.clone();item.metadata_status.push_str(&format!("\n{}",candidate.metadata_status));item})).collect();
                    }
                    if ui.get_page() == 0 && ui.get_search().is_empty() { items.retain(|i| i.kind == ItemKind::World); items.sort_by_key(|i| std::cmp::Reverse(i.last_played_ms.or(i.modified_ms))); }
                    let new_filters: Vec<_> = std::iter::once(None).chain(games.iter().map(|g| g.game_dir.as_ref().map(|p| display_path(p)))).collect();
                    if new_filters != state.filters {
                        let previous = state.filters.get(ui.get_filter_index().max(0) as usize).cloned().flatten();
                        ui.set_filters(strings(std::iter::once("全部游戏目录".into()).chain(games.iter().map(|g| format!("{} · {}",g.name,display_path(&g.path)))).collect()));
                        ui.set_filter_index(new_filters.iter().position(|f| f == &previous).unwrap_or(0) as i32);
                        if new_filters.get(ui.get_filter_index().max(0) as usize).cloned().flatten()!=previous {state.query_due=Some(Instant::now());}
                        state.filters = new_filters;
                    }
                    state.versions = versions; state.games = games;
                    ui.set_game_labels(strings(state.games.iter().map(|g| format!("{} · {}",g.ownership,display_path(&g.path))).collect()));
                    ui.set_version_labels(strings(state.versions.iter().map(|v| format!("{} · {}",v.name,display_path(&v.path))).collect()));
                    ui.set_counts(format!("{} 个来源 / {} 个版本 / {} 个游戏目录 / {} 张存档\n{} 个资源包 / {} 个模组 / {} 个光影包 / {} 个数据包",state.settings.sources.len(),stats.versions,stats.instances,stats.worlds,stats.resource_packs,stats.mods,stats.shader_packs,stats.data_packs).into());
                    ui.set_summary(format!("已索引世界逻辑大小：{}。包含缓存与可能的大小下限；世界和游戏目录大小不可相加。", bytes_label(Some(stats.world_bytes),false)).into());
                    let rows=items.iter().map(|i| {let key=icon_key(i);EntryRow {
                        id: i.id.clone().into(), title: format!("{}{}",if i.favorite {"★ "}else {""},i.name).into(), kind: i.kind.label().into(), owner: i.ownership.clone().into(),
                        path: display_path(&i.path).into(), size: bytes_label(i.size,i.size_complete).into(),
                        time: time_label(i.last_played_ms.or(i.modified_ms)).into(), available: i.available,
                        icon: poll_icons.borrow().loaded.get(&key).cloned().flatten().unwrap_or_default(), icon_key:key.into(),
                    }}).collect::<Vec<_>>();
                    state.row_positions=items.iter().enumerate().map(|(n,i)|(i.id.clone(),n)).collect();
                    poll_icons.borrow_mut().sources=items.iter().map(|i|(i.id.clone(),IconRequest{id:i.id.clone(),key:icon_key(i),path:i.path.clone(),kind:i.kind.clone(),icon:i.icon.clone()})).collect();
                    state.row_model=Rc::new(VecModel::from(rows));
                    ui.set_entries(state.row_model.clone().into());
                    if let Some(item) = items.iter().find(|i| i.id == ui.get_selected_id().as_str()) {
                        let annotations_changed=state.selected.as_ref().is_none_or(|old|old.id!=item.id || old.favorite!=item.favorite || old.tags!=item.tags);
                        let key=icon_key(item);
                        let icon_changed=state.selected.as_ref().is_none_or(|old|icon_key(old)!=key);
                        state.selected = Some(item.clone()); detail(&ui,&state,item);
                        if icon_changed {
                            let cached=poll_icons.borrow().loaded.get(&key).cloned();
                            ui.set_has_icon(cached.as_ref().is_some_and(|i|i.is_some()));
                            if let Some(Some(icon))=cached {ui.set_detail_icon(icon);}
                            else {ui.invoke_request_row_icon(item.id.clone().into(),key.into());}
                        }
                        if annotations_changed {ui.set_detail_favorite(item.favorite);ui.set_detail_tags(item.tags.join(", ").into());}
                    }
                    else if !ui.get_selected_id().is_empty() { ui.set_selected_id("".into()); state.selected = None; }
                    state.items = items;
                }
                Event::Status(message) => ui.set_status(message.into()),
                Event::Progress(progress,message) => { ui.set_progress(progress); ui.set_status(message.into()); }
                Event::Scanning(scanning) => { ui.set_scanning(scanning); if !scanning { ui.set_progress(0.0); } else {poll_icons.borrow_mut().loaded.retain(|_,icon|icon.is_some());} }
                Event::Dirty => { if state.query_due.is_none() { state.query_due = Some(Instant::now() + Duration::from_millis(100)); } }
                Event::Warning(warning) => { state.diagnostics.push(warning); if state.diagnostics.len() > 100 { state.diagnostics.remove(1); } }
                Event::Detail(item, icon) => { let item = *item; if item.id == ui.get_selected_id().as_str() && state.selected.as_ref().is_none_or(|current|icon_key(current)==icon_key(&item)) {
                    state.selected = Some(item.clone()); detail(&ui,&state,&item);
                    ui.set_has_icon(icon.is_some());
                    if let Some((pixels,width,height)) = icon { let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&pixels,width,height); ui.set_detail_icon(slint::Image::from_rgba8(buffer)); }
                } else if item.id == ui.get_selected_id().as_str() {state.query_due=Some(Instant::now());}}
                Event::Browse(field,path) => match field {
                    0 => { ui.set_edit_executable(display_path(&path).into()); if ui.get_edit_workdir().is_empty() && let Some(parent) = path.parent() { ui.set_edit_workdir(display_path(parent).into()); } }
                    1 => ui.set_edit_workdir(display_path(&path).into()),
                    _ => ui.set_edit_java(display_path(&path).into()),
                },
                Event::EditorSaved => { ui.set_editor_visible(false); ui.set_mapping_visible(false); launch_info(&ui,&state); }
                Event::SourcesChanged => { let _ = poll_sender.send(Command::Scan); }
                Event::Duplicates(items, summary) => {state.duplicate_items=items;ui.set_duplicate_summary(summary.into());state.query_due=Some(Instant::now());}
                Event::RowIcon(id,key,pixels) => {
                    let icon=pixels.map(|(pixels,width,height)|slint::Image::from_rgba8(slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&pixels,width,height)));
                    { let mut cache=poll_icons.borrow_mut();cache.pending.remove(&key);
                      cache.loaded.insert(key.clone(),icon.clone()); cache.order.retain(|old|old!=&key);cache.order.push_back(key.clone());
                      while cache.order.len()>256 {if let Some(old)=cache.order.pop_front() {cache.loaded.remove(&old);}} }
                    if let Some(&position)=state.row_positions.get(&id) && let Some(mut row)=state.row_model.row_data(position) && row.icon_key==key {
                        row.icon=icon.clone().unwrap_or_default();state.row_model.set_row_data(position,row);
                    }
                    if ui.get_selected_id()==id && state.selected.as_ref().is_some_and(|item|icon_key(item)==key) {
                        ui.set_has_icon(icon.is_some());if let Some(icon)=icon {ui.set_detail_icon(icon);}
                    }
                }
            }
        }
        { let mut cache=poll_icons.borrow_mut(); while let Some(request)=cache.deferred.pop_front() {
            match icon_sender.try_send(request) {Ok(())=>(),Err(mpsc::TrySendError::Full(request))=>{cache.deferred.push_front(request);break;},Err(mpsc::TrySendError::Disconnected(request))=>{cache.pending.remove(&request.key);} }
        } }
        ui.set_diagnostics(state.diagnostics.join("\n\n").into());
        if state.query_due.is_some_and(|due| Instant::now() >= due) { queue_query(&ui,&mut state,&poll_sender); }
    });
    #[cfg(feature = "ui-smoke")]
    let _smoke_timer = crate::ui_smoke::start(&ui);
    #[cfg(feature = "ui-smoke")]
    let _stage2_smoke_timer = crate::ui_stage2_smoke::start(&ui);
    ui.run()?;
    cancel.store(true, Ordering::SeqCst);
    timer.stop();
    Ok(())
}
