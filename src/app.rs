use crate::{AppWindow, EntryRow, I18n, LauncherRow, SourceRow, SpaceBar, SpaceTile};
use anyhow::{Context, Result, bail};
use minecraft_explorer::{
    content, duplicates, i18n,
    index::Index,
    launcher,
    models::*,
    operations::{self, Action, Conflict, Plan, Policy, Report, VersionScope},
    platform::*,
    scanner::{self, ScanEvent},
    space, watch,
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
    ScanAdded(PathBuf, u64),
    Annotate(String, bool, Vec<String>),
    Watch(bool),
    Appearance(bool, bool, u64),
    Analyze(Vec<PathBuf>, u64),
    Duplicates,
    AddSource(SourceKind, Option<PathBuf>),
    RemoveSource(String),
    SaveLauncher(LauncherInstallation),
    RemoveLauncher(String),
    Map(String, String, String, PathBuf, Option<LaunchScript>),
    ClearMap(String),
    DefaultLauncher(String),
    Open(String, String),
    LaunchScript(String, String),
    Reveal(String),
    RevealPath(PathBuf),
    ExportLicenses,
    DiscoverScript(String, String, String, PathBuf, String, u64),
    Copy(String),
    Detail(String),
    Browse(i32),
    PrepareOperation(
        Vec<String>,
        Action,
        PathBuf,
        String,
        Conflict,
        VersionScope,
        PathBuf,
        String,
    ),
    ExecuteOperation(Box<Plan>),
    OperationComplete(Box<Report>, TaskState),
    OperationFailed(Vec<PathBuf>, String, TaskState),
    History,
    RevealHistory,
    BrowseOperation(i32),
}

enum Event {
    Settings(Settings),
    AppearanceSaved(u64),
    ScriptDiscovered(Option<LaunchScript>, String, String, u64),
    SourceAdded(PathBuf, u64),
    Analysis(space::Analysis, u64),
    SpaceStatus(String, u64),
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
    OperationPrepared(Box<Plan>, String),
    OperationError(String),
    OperationHistory(String),
    OperationBrowse(i32, PathBuf),
    OperationFinished,
}

#[derive(Default)]
struct UiState {
    script_request: u64,
    script_initial: (String, String),
    settings: Settings,
    appearance_request: u64,
    appearance_pending: bool,
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
    checked: HashSet<String>,
    operation_ids: Vec<String>,
    operation_plan: Option<Plan>,
    operation_signature: String,
    close_requested: bool,
    analysis: Option<space::Analysis>,
    analysis_request: u64,
    auto_analysis_request: Option<u64>,
    auto_analysis_path: Option<PathBuf>,
    assigned_space_index: Option<i32>,
    space_node: usize,
    space_parents: Vec<usize>,
    space_scopes: Vec<Vec<PathBuf>>,
    raw_status: String,
    raw_history: String,
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
    callback: mpsc::Sender<Command>,
    closing: Arc<AtomicBool>,
) {
    let result = (|| -> Result<()> {
        let index = Index::open(&db)?;
        let mut settings = index.load_settings()?;
        let scanning = Arc::new(AtomicBool::new(false));
        let mut children = Vec::<launcher::LauncherProcess>::new();
        i18n::warm_up();
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
        let mut pending_added = None;
        let mut watch_retry = Instant::now();
        let mut watch_incomplete = false;
        loop {
            if closing.load(Ordering::SeqCst) && !scanning.load(Ordering::SeqCst) {
                break;
            }
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
            let command = if !closing.load(Ordering::SeqCst)
                && !scanning.load(Ordering::SeqCst)
                && pending_added.is_some()
            {
                let (path, request) = pending_added.take().unwrap();
                Command::ScanAdded(path, request)
            } else if !closing.load(Ordering::SeqCst)
                && !scanning.load(Ordering::SeqCst)
                && pending_full
            {
                pending_full = false;
                pending_paths.clear();
                Command::Scan
            } else if !closing.load(Ordering::SeqCst)
                && !scanning.load(Ordering::SeqCst)
                && !pending_paths.is_empty()
            {
                Command::ScanPaths(std::mem::take(&mut pending_paths))
            } else {
                match commands.recv_timeout(Duration::from_millis(200)) {
                    Ok(command) => command,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            };
            children.retain_mut(|child| matches!(child.try_wait(), Ok(None)));
            let file_request = matches!(
                &command,
                Command::PrepareOperation(..)
                    | Command::ExecuteOperation(..)
                    | Command::OperationComplete(..)
                    | Command::OperationFailed(..)
            );
            let analysis_request = match &command {
                Command::Analyze(_, request) | Command::ScanAdded(_, request) => Some(*request),
                _ => None,
            };
            let result = (|| -> Result<()> {
                if closing.load(Ordering::SeqCst)
                    && matches!(
                        &command,
                        Command::PrepareOperation(..)
                            | Command::ExecuteOperation(..)
                            | Command::Scan
                            | Command::ScanPaths(..)
                            | Command::ScanAdded(..)
                    )
                {
                    bail!("窗口正在关闭，未开始新的任务");
                }
                if scanning.load(Ordering::SeqCst)
                    && matches!(
                        &command,
                        Command::SaveLauncher(..)
                            | Command::RemoveLauncher(..)
                            | Command::Map(..)
                            | Command::ClearMap(..)
                            | Command::DefaultLauncher(..)
                    )
                {
                    bail!("请等待或取消任务后再修改启动器或实例配置");
                }
                match command {
                    Command::Appearance(english, light_theme, request) => {
                        settings.english = english;
                        settings.light_theme = light_theme;
                        index.save_settings(&settings)?;
                        let _ = events.send(Event::AppearanceSaved(request));
                    }
                    Command::Analyze(paths, request) => {
                        if scanning.swap(true, Ordering::SeqCst) {
                            let _ = events.send(Event::SpaceStatus(
                                "当前任务尚未结束，请完成或取消后重新分析".into(),
                                request,
                            ));
                            bail!("请等待或取消当前任务");
                        }
                        cancel.store(false, Ordering::SeqCst);
                        let guard = TaskState {
                            running: scanning.clone(),
                            events: events.clone(),
                        };
                        let items = index.query(&Query::default())?;
                        let flag = cancel.clone();
                        let output = events.clone();
                        let _ = events.send(Event::Scanning(true));
                        std::thread::Builder::new()
                            .name("space-analysis".into())
                            .spawn(move || {
                                let _guard = guard;
                                let started = Instant::now();
                                let mut updated = Instant::now() - Duration::from_secs(1);
                                match space::analyze(&paths, &items, &flag, |files| {
                                    if updated.elapsed() >= Duration::from_millis(250) {
                                        let _ = output.send(Event::SpaceStatus(format!(
                                            "空间分析：已读取 {files} 个文件，用时 {seconds} 秒", seconds = started.elapsed().as_secs()
                                        ), request));
                                        updated = Instant::now();
                                    }
                                }) {
                                    Ok(tree) => {
                                        let _ = output.send(Event::Analysis(tree, request));
                                    }
                                    Err(error) => {
                                        let _ = output.send(Event::SpaceStatus(format!(
                                            "空间分析失败：{error:#}"
                                        ), request));
                                    }
                                }
                            })?;
                    }
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
                    Command::Scan | Command::ScanPaths(_) | Command::ScanAdded(..) => {
                        let auto_analysis = match &command {
                            Command::ScanAdded(path, request) => Some((path.clone(), *request)),
                            _ => None,
                        };
                        let paths = if let Command::ScanPaths(paths) = command {
                            Some(paths)
                        } else {
                            None
                        };
                        let full = paths.is_none();
                        if cancel.load(Ordering::SeqCst)
                            && let Some((_, request)) = auto_analysis.as_ref()
                        {
                            let _ = events.send(Event::SpaceStatus(
                                "自动空间分析已取消；目录关联仍保留".into(),
                                *request,
                            ));
                            let _ = events.send(Event::Scanning(false));
                            return Ok(());
                        }
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
                                let auto_request = auto_analysis.as_ref().map(|(_, request)| *request);
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
                                            ScanEvent::Discovered(items) => (|| -> Result<()> {
                                                if full {
                                                    let cached = db.query(&Query {
                                                        kind: Some(ItemKind::DataPack),
                                                        ..Default::default()
                                                    })?;
                                                    db.remove_items(
                                                        &scanner::invalid_data_pack_cache(
                                                            &cached, &items,
                                                        ),
                                                    )?;
                                                }
                                                db.discovered(&items, generation)
                                            })(
                                            ),
                                            ScanEvent::Metadata(mut item) => {
                                                if let Ok(Some(old)) = db.get(&item.id) {
                                                    item.size = old.size;
                                                    item.size_complete = old.size_complete;
                                                }
                                                db.upsert(&item, generation)
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
                                    if let Some((path, request)) = auto_analysis {
                                        if report.cancelled || scan_cancel.load(Ordering::SeqCst) {
                                            let _ = output.send(Event::SpaceStatus("自动空间分析已取消；目录关联仍保留".into(), request));
                                        } else {
                                            let _ = output.send(Event::SpaceStatus("正在自动分析新增目录…".into(), request));
                                            let items = db.query(&Query::default())?;
                                            let began = Instant::now();
                                            let mut updated = Instant::now() - Duration::from_secs(1);
                                            let tree = space::analyze(&[path], &items, &scan_cancel, |files| {
                                                if updated.elapsed() >= Duration::from_millis(250) {
                                                    let _ = output.send(Event::SpaceStatus(format!("空间分析：已读取 {files} 个文件，用时 {seconds} 秒", seconds = began.elapsed().as_secs()), request));
                                                    updated = Instant::now();
                                                }
                                            })?;
                                            let _ = output.send(Event::Analysis(tree, request));
                                        }
                                    }
                                    Ok(report.cancelled)
                                })();
                                if let Err(error) = result {
                                    if let Some(request) = auto_request {
                                        let _ = output.send(Event::SpaceStatus(format!("空间分析失败：{error:#}"), request));
                                    }
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
                        #[cfg(feature = "ui-smoke")]
                        let preset = crate::ui_stage4_smoke::chosen_source();
                        #[cfg(not(feature = "ui-smoke"))]
                        let preset: Option<PathBuf> = None;
                        let selected = preset.or_else(|| {
                            rfd::FileDialog::new()
                                .set_title(i18n::translate(
                                    "选择实际的 Minecraft 根目录 / 游戏目录",
                                    settings.english,
                                ))
                                .pick_folder()
                        });
                        if let Some(path) = selected {
                            let path = checked_path(&path)?;
                            if settings
                                .sources
                                .iter()
                                .any(|s| path_key(&s.path) == path_key(&path))
                            {
                                bail!("该目录已添加");
                            }
                            settings.sources.push(Source {
                                path: path.clone(),
                                kind,
                                associated_root,
                            });
                            index.save_settings(&settings)?;
                            let _ = events.send(Event::Settings(settings.clone()));
                            let request = now_ms() as u64;
                            cancel.store(false, Ordering::SeqCst);
                            pending_added = Some((path.clone(), request));
                            let _ = events
                                .send(Event::Status("目录已添加；索引刷新后自动分析该目录".into()));
                            let _ = events.send(Event::SourceAdded(path, request));
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
                    Command::Map(subject_id, launcher_id, version_id, game_dir, script) => {
                        let subject = index.get(&subject_id)?.context("所选条目已失效")?;
                        let version = index.get(&version_id)?.context("目标版本已失效")?;
                        let installation = settings
                            .launchers
                            .iter()
                            .find(|l| l.id == launcher_id)
                            .context("请选择启动器")?;
                        let mut mapping =
                            launcher::create_mapping(&subject, installation, &version, &game_dir)?;
                        if let Some(script) = &script {
                            launcher::validate_script(script)?;
                        }
                        mapping.script = script;
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
                    Command::LaunchScript(subject_id, launcher_id) => {
                        let installation = settings
                            .launchers
                            .iter()
                            .find(|l| l.id == launcher_id)
                            .context("启动器未配置")?;
                        let subject = index.get(&subject_id)?.context("所选条目已失效")?;
                        let mapping = resolve_mapping(&settings, &subject, &launcher_id)
                            .context("请先绑定该版本的启动脚本")?;
                        let version = index
                            .get(&format!("version:{}", path_key(&mapping.version_path)))?
                            .context("目标版本已失效")?;
                        let mut child = launcher::spawn_script(installation, mapping, &version)?;
                        std::thread::sleep(Duration::from_millis(250));
                        if let Some(exit) = child.try_wait()?
                            && !exit.success()
                        {
                            bail!("启动脚本退出失败：{exit}");
                        }
                        children.push(child);
                        index.record_opened(&subject_id)?;
                        let _ = events.send(Event::Dirty);
                        let _ = events.send(Event::Status(format!(
                            "已运行关联脚本：{id}；请检查游戏窗口，未确认游戏进程",
                            id = mapping.version_id
                        )));
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
                    Command::RevealPath(path) => {
                        reveal(&path)?;
                        let _ = events.send(Event::Status("已向文件资源管理器发送定位请求".into()));
                    }
                    Command::ExportLicenses => {
                        let path = minecraft_explorer::licenses::export(
                            db.parent().context("数据目录缺失")?,
                        )?;
                        reveal(&path)?;
                        let _ = events.send(Event::Status(
                            "已导出许可文件，请在资源管理器中双击查看".into(),
                        ));
                    }
                    Command::DiscoverScript(
                        subject_id,
                        launcher_id,
                        version_id,
                        game_dir,
                        signature,
                        request,
                    ) => {
                        let result = (|| -> Result<Option<LaunchScript>> {
                            let subject = index.get(&subject_id)?.context("所选条目已失效")?;
                            let version = index.get(&version_id)?.context("目标版本已失效")?;
                            let installation = settings
                                .launchers
                                .iter()
                                .find(|l| l.id == launcher_id)
                                .context("请选择启动器")?;
                            let mapping = launcher::create_mapping(
                                &subject,
                                installation,
                                &version,
                                &game_dir,
                            )?;
                            minecraft_explorer::launch_scripts::discover(
                                installation,
                                &mapping,
                                db.parent().context("数据目录缺失")?,
                            )
                        })();
                        let (script, notice) = match result {
                            Ok(Some(script)) => (Some(script), "已识别匹配脚本并保存独立副本；保存关联后可启动，登录失效时需在原启动器重新运行并识别。".to_string()),
                            Ok(None) => (None, "未找到版本和游戏目录均匹配的 PCL 脚本。请先在所选启动器启动这个版本一次，再点自动识别；也可手动选择导出脚本。".to_string()),
                            Err(_) => (None, "无法自动识别：请核对目标版本、游戏目录与启动器安装，或手动选择脚本。".to_string()),
                        };
                        let _ = events
                            .send(Event::ScriptDiscovered(script, signature, notice, request));
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
                    Command::PrepareOperation(
                        ids,
                        action,
                        parent,
                        name,
                        conflict,
                        scope,
                        backup,
                        signature,
                    ) => {
                        if scanning.swap(true, Ordering::SeqCst) {
                            bail!("请等待或取消当前任务");
                        }
                        cancel.store(false, Ordering::SeqCst);
                        let _ = events.send(Event::Scanning(true));
                        let task = TaskState {
                            running: scanning.clone(),
                            events: events.clone(),
                        };
                        let selected = ids
                            .iter()
                            .map(|id| index.get(id)?.context("所选条目已失效，请刷新"))
                            .collect::<Result<Vec<_>>>()?;
                        let games = index.query(&Query {
                            kind: Some(ItemKind::Instance),
                            ..Default::default()
                        })?;
                        let policy =
                            Policy::new(&settings, &games, db.parent().context("数据目录缺失")?)?;
                        let flag = cancel.clone();
                        let output = events.clone();
                        std::thread::Builder::new()
                            .name("operation-preview".into())
                            .spawn(move || {
                                let _task = task;
                                let _ = output.send(Event::Status(
                                    "正在检查真实路径、文件数和完整哈希…".into(),
                                ));
                                let result = if action == Action::Restore {
                                    operations::prepare_restore(
                                        &policy, &backup, &parent, conflict, &flag,
                                    )
                                } else {
                                    operations::prepare(
                                        &policy,
                                        &selected,
                                        action,
                                        (!parent.as_os_str().is_empty())
                                            .then_some(parent.as_path()),
                                        &name,
                                        conflict,
                                        scope,
                                        &flag,
                                    )
                                };
                                match result {
                                    Ok(plan) => {
                                        let _ = output.send(Event::OperationPrepared(
                                            Box::new(plan),
                                            signature,
                                        ));
                                    }
                                    Err(e) => {
                                        let _ = output.send(Event::OperationError(format!(
                                            "预览失败：{e:#}"
                                        )));
                                    }
                                }
                            })
                            .context("无法创建操作预览线程")?;
                    }
                    Command::ExecuteOperation(plan) => {
                        if scanning.swap(true, Ordering::SeqCst) {
                            bail!("索引正在变化，请重新预览后再执行");
                        }
                        cancel.store(false, Ordering::SeqCst);
                        let _ = events.send(Event::Scanning(true));
                        let task = TaskState {
                            running: scanning.clone(),
                            events: events.clone(),
                        };
                        let games = index.query(&Query {
                            kind: Some(ItemKind::Instance),
                            ..Default::default()
                        })?;
                        let policy =
                            Policy::new(&settings, &games, db.parent().context("数据目录缺失")?)?;
                        let flag = cancel.clone();
                        let output = events.clone();
                        let completed = callback.clone();
                        std::thread::Builder::new()
                            .name("file-operation".into())
                            .spawn(move || {
                                let paths = plan
                                    .units
                                    .iter()
                                    .flat_map(|u| {
                                        std::iter::once(u.source.clone())
                                            .chain(u.destination.clone())
                                    })
                                    .collect();
                                let result = operations::execute(
                                    &policy,
                                    *plan,
                                    &flag,
                                    |done, total, message| {
                                        let _ = output.send(Event::Progress(
                                            done as f32 / total.max(1) as f32,
                                            message,
                                        ));
                                    },
                                );
                                match result {
                                    Ok(report) => {
                                        let _ = completed.send(Command::OperationComplete(
                                            Box::new(report),
                                            task,
                                        ));
                                    }
                                    Err(e) => {
                                        let _ = completed.send(Command::OperationFailed(
                                            paths,
                                            format!(
                                                "文件操作失败：{e:#}；请查看操作记录和真实路径"
                                            ),
                                            task,
                                        ));
                                    }
                                }
                            })
                            .context("无法创建文件操作线程")?;
                    }
                    Command::OperationComplete(report, task) => {
                        let _task = task;
                        let mut paths = Vec::new();
                        let mut messages = Vec::new();
                        for outcome in &report.outcomes {
                            paths.push(outcome.source.clone());
                            if let Some(target) = &outcome.destination {
                                paths.push(target.clone());
                            }
                            messages.push(format!(
                                "{}：{}",
                                display_path(&outcome.source),
                                outcome.message
                            ));
                            if outcome.success
                                && matches!(report.plan.action, Action::Rename | Action::Move)
                                && let Some(target) = &outcome.destination
                            {
                                if let Err(e) = index.relocate_annotations(&outcome.source, target)
                                {
                                    messages.push(format!("文件已完成，但注释同步失败：{e:#}"));
                                }
                                if let Some(unit) = report
                                    .plan
                                    .units
                                    .iter()
                                    .find(|u| u.source == outcome.source)
                                {
                                    let new_id =
                                        format!("{}:{}", unit.kind.as_str(), path_key(target));
                                    if report.plan.action == Action::Rename {
                                        for mapping in &mut settings.mappings {
                                            if mapping.subject_id == unit.item_id {
                                                mapping.subject_id = new_id.clone();
                                            }
                                        }
                                        if let Some(default) =
                                            settings.instance_defaults.remove(&unit.item_id)
                                        {
                                            settings.instance_defaults.insert(new_id, default);
                                        }
                                    } else {
                                        settings.mappings.retain(|m| m.subject_id != unit.item_id);
                                        settings.instance_defaults.remove(&unit.item_id);
                                    }
                                }
                            }
                        }
                        index.save_settings(&settings)?;
                        pending_paths.extend(paths);
                        let policy = Policy::new(
                            &settings,
                            &index.query(&Query {
                                kind: Some(ItemKind::Instance),
                                ..Default::default()
                            })?,
                            db.parent().context("数据目录缺失")?,
                        )?;
                        let _ = events.send(Event::OperationHistory(operations::history(&policy)?));
                        let _ = events.send(Event::Settings(settings.clone()));
                        if report.finished {
                            let _ = events.send(Event::OperationFinished);
                        } else {
                            let _ = events.send(Event::OperationError(messages.join("\n\n")));
                        }
                        let success = report.outcomes.iter().filter(|o| o.success).count();
                        let _ = events.send(Event::Status(format!(
                            "{}：{} / {} 项完成；{}",
                            report.plan.action.label(),
                            success,
                            report.plan.units.len(),
                            messages.join("；")
                        )));
                        for message in messages {
                            let _ = events.send(Event::Warning(message));
                        }
                    }
                    Command::OperationFailed(paths, message, task) => {
                        let _task = task;
                        pending_paths.extend(paths);
                        let _ = events.send(Event::OperationError(message));
                    }
                    Command::History | Command::RevealHistory => {
                        let policy = Policy::new(
                            &settings,
                            &index.query(&Query {
                                kind: Some(ItemKind::Instance),
                                ..Default::default()
                            })?,
                            db.parent().context("数据目录缺失")?,
                        )?;
                        if matches!(command, Command::RevealHistory) {
                            std::fs::create_dir_all(&policy.records)?;
                            reveal(&policy.records)?;
                        }
                        let _ = events.send(Event::OperationHistory(operations::history(&policy)?));
                    }
                    Command::BrowseOperation(field) => {
                        if let Some(path) = rfd::FileDialog::new()
                            .set_title(i18n::translate(
                                if field == 1 {
                                    "选择本程序的目录备份"
                                } else {
                                    "选择目标父目录"
                                },
                                settings.english,
                            ))
                            .pick_folder()
                        {
                            let _ = events.send(Event::OperationBrowse(field, path));
                        }
                    }
                    Command::Browse(field) => {
                        let dialog = rfd::FileDialog::new();
                        let path = if field == 1 || field == 4 {
                            dialog.pick_folder()
                        } else if field == 3 {
                            dialog.add_filter("BAT / CMD", &["bat", "cmd"]).pick_file()
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
                if let Some(request) = analysis_request {
                    let _ = events.send(Event::SpaceStatus(
                        format!("空间分析失败：{error:#}"),
                        request,
                    ));
                }
                let _ = events.send(Event::Status(format!("操作失败：{error:#}")));
                if file_request {
                    let _ = events.send(Event::OperationError(format!("操作失败：{error:#}")));
                }
            }
        }
        cancel.store(true, Ordering::SeqCst);
        let _ = events.send(Event::Scanning(false));
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
    let size = i18n::translate(
        &bytes_label(item.size, item.size_complete),
        ui.get_english(),
    );
    ui.set_detail_text(format!("{}\n\n真实路径\n{}\n\n实际游戏目录\n{}\n\n归属\n{}\n\n版本定义 ID\n{}\n\n世界声明的历史版本\n{}\n\n逻辑大小\n{}\n\n最后游玩\n{}\n修改时间\n{}\n\n信息来源 / 状态\n{}{}", i18n::translate(item.kind.label(),ui.get_english()), display_path(&item.path),
        item.game_dir.as_ref().map(|p| display_path(p)).unwrap_or_else(|| i18n::translate("未确认；请手动关联",ui.get_english())), i18n::translate(&item.ownership,ui.get_english()),
        item.version_id.clone().unwrap_or_else(|| i18n::translate("未知 / 共享",ui.get_english())), item.game_version.clone().unwrap_or_else(|| i18n::translate("未知",ui.get_english())), size,
        i18n::translate(&time_label(item.last_played_ms),ui.get_english()), i18n::translate(&time_label(item.modified_ms),ui.get_english()), i18n::translate(&item.metadata_status,ui.get_english()),
        i18n::translate(if item.available { "" } else { "\n来源离线：当前显示缓存" },ui.get_english())).into());
    if let Some(meta) = &item.content {
        let extra = i18n::translate(
            &format!(
                "\n\n内容声明信息\nID：{}\n版本：{}\n描述：{}\n作者：{}\nLoader：{}\n兼容性：{}",
                meta.declared_id.as_deref().unwrap_or("未知"),
                meta.declared_version.as_deref().unwrap_or("未知"),
                meta.description.as_deref().unwrap_or("未提供"),
                meta.authors.join("、"),
                meta.loaders.join("、"),
                meta.compatibility.join("、")
            ),
            ui.get_english(),
        );
        ui.set_detail_text(format!("{}{extra}", ui.get_detail_text()).into());
    }
    launch_info(ui, state);
    ui.set_detail_text(i18n::translate(&ui.get_detail_text(), ui.get_english()).into());
}

fn script_signature(ui: &AppWindow, state: &UiState) -> Option<String> {
    let launcher = selected_launcher(ui, state)?;
    let version = state.versions.get(ui.get_version_index().max(0) as usize)?;
    let game = state
        .games
        .get(ui.get_mapping_game_index().max(0) as usize)?;
    Some(format!(
        "{}|{}|{}|{}",
        ui.get_selected_id(),
        launcher.id,
        version.id,
        path_key(&game.path)
    ))
}

fn queue_script_discovery(ui: &AppWindow, state: &mut UiState, sender: &mpsc::Sender<Command>) {
    if !ui.get_mapping_visible() {
        return;
    }
    let Some(signature) = script_signature(ui, state) else {
        return;
    };
    state.script_request += 1;
    state.script_initial = (
        ui.get_mapping_script().to_string(),
        ui.get_mapping_script_workdir().to_string(),
    );
    let launcher = selected_launcher(ui, state).unwrap();
    let version = &state.versions[ui.get_version_index().max(0) as usize];
    let game = &state.games[ui.get_mapping_game_index().max(0) as usize];
    ui.set_mapping_script_notice(
        i18n::translate("正在检查所选启动器的上次启动脚本…", ui.get_english()).into(),
    );
    let _ = sender.send(Command::DiscoverScript(
        ui.get_selected_id().to_string(),
        launcher.id.clone(),
        version.id.clone(),
        game.path.clone(),
        signature,
        state.script_request,
    ));
}

fn selected_launcher<'a>(ui: &AppWindow, state: &'a UiState) -> Option<&'a LauncherInstallation> {
    state
        .settings
        .launchers
        .get(ui.get_launcher_index().max(0) as usize)
}

fn launch_info(ui: &AppWindow, state: &UiState) {
    ui.set_can_launch_script(false);
    let Some(item) = &state.selected else {
        return;
    };
    let Some(installation) = selected_launcher(ui, state) else {
        ui.set_launch_status("LauncherNotConfigured".into());
        ui.set_launch_hint(
            i18n::translate(
                "在设置中添加 PCL2 / PCL CE / HMCL 安装文件",
                ui.get_english(),
            )
            .into(),
        );
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
    ui.set_can_launch_script(
        same_root && valid_mapping && item.available && mapping.is_some_and(|m| m.script.is_some()),
    );
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
        } else if let Some(script) = mapping.and_then(|m| m.script.as_ref()) {
            format!("已绑定启动脚本：{path}\n目标版本：{id}\n工作目录：{directory}", path = display_path(&script.path), id = target, directory = display_path(&script.working_dir))
        } else if target.is_empty() {
            "共享目录需手动关联目标版本；在设置关联中可绑定该版本的启动脚本。".to_string()
        } else {
            format!(
                "打开 {} 后手动选择：{}\n根目录：{}\n如需直接启动，请在设置关联中绑定该版本的启动脚本。",
                installation.kind.label(),
                target,
                display_path(&installation.root)
            )
        }
        .into(),
    );
    ui.set_launch_hint(i18n::translate(&ui.get_launch_hint(), ui.get_english()).into());
}

fn apply_settings(ui: &AppWindow, state: &mut UiState, settings: Settings) {
    let previous = selected_launcher(ui, state).map(|l| l.id.clone());
    state.settings = settings;
    ui.set_english(state.settings.english);
    ui.set_light_theme(state.settings.light_theme);
    ui.set_filters(strings(
        std::iter::once(i18n::translate("全部游戏目录", ui.get_english()))
            .chain(
                state
                    .games
                    .iter()
                    .map(|g| format!("{} · {}", g.name, display_path(&g.path))),
            )
            .collect(),
    ));
    ui.set_status(i18n::translate(&state.raw_status, ui.get_english()).into());
    ui.set_operation_history(i18n::translate(&state.raw_history, ui.get_english()).into());
    if let Some(plan) = &state.operation_plan {
        ui.set_operation_preview(plan.preview_localized(ui.get_english()).into());
    }
    ui.set_watch_enabled(state.settings.watch_enabled);
    ui.set_source_labels(strings(
        std::iter::once(i18n::translate("全部来源 / 根目录", ui.get_english()))
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
                    label: i18n::translate(
                        if s.kind == SourceKind::Root {
                            "Minecraft 根目录"
                        } else {
                            "自定义游戏目录"
                        },
                        ui.get_english(),
                    )
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
                    label: i18n::translate(
                        &format!(
                            "{}{} · OpenOnly",
                            l.kind.label(),
                            if state.settings.default_launcher.as_ref() == Some(&l.id) {
                                if ui.get_english() {
                                    " (default)"
                                } else {
                                    "（默认）"
                                }
                            } else {
                                ""
                            }
                        ),
                        ui.get_english(),
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
    if let Some(item) = &state.selected {
        detail(ui, state, item);
    }
    update_space_scopes(ui, state);
    render_space(ui, state);
}

fn render_space(ui: &AppWindow, state: &UiState) {
    let Some(analysis) = &state.analysis else {
        return;
    };
    let english = ui.get_english();
    let node = &analysis.nodes[state.space_node];
    let colors = [
        (49, 124, 86),
        (55, 113, 166),
        (142, 91, 173),
        (182, 117, 52),
        (48, 135, 139),
        (93, 107, 125),
    ];
    let tint = |category: usize| {
        let (r, g, b) = colors[category];
        slint::Color::from_rgb_u8(r, g, b)
    };
    let tiles = analysis
        .layout(state.space_node, 1.8)
        .into_iter()
        .map(|tile| {
            let entry = tile.node.map(|i| &analysis.nodes[i]);
            let size = bytes_label(Some(tile.bytes), node.complete);
            let title = entry
                .map(|n| n.name.clone())
                .unwrap_or_else(|| i18n::translate("其他小项 / 汇总", english));
            let info = entry
                .map(|n| {
                    format!(
                        "{} · {}\n{}\n{}",
                        n.name,
                        size,
                        n.path.as_ref().map(|p| display_path(p)).unwrap_or_default(),
                        n.owner
                    )
                })
                .unwrap_or_else(|| format!("{title} · {size}"));
            SpaceTile {
                node: tile.node.map_or(-1, |n| n as i32),
                title: title.into(),
                info: info.into(),
                size: size.into(),
                x: tile.x,
                y: tile.y,
                w: tile.width,
                h: tile.height,
                tint: tint(tile.category),
                directory: entry.is_some_and(|n| n.directory && !n.children.is_empty()),
                item_id: entry
                    .and_then(|n| n.item_id.clone())
                    .unwrap_or_default()
                    .into(),
            }
        })
        .collect::<Vec<_>>();
    ui.set_space_tiles(Rc::new(VecModel::from(tiles)).into());
    let names = ["存档", "资源包", "模组", "光影包", "数据包", "其他文件"];
    let total = analysis.nodes[0].bytes;
    ui.set_space_bytes(total.to_string().into());
    ui.set_space_bars(
        Rc::new(VecModel::from(
            analysis
                .categories
                .iter()
                .enumerate()
                .map(|(i, bytes)| SpaceBar {
                    title: i18n::translate(names[i], english).into(),
                    size: bytes_label(Some(*bytes), analysis.nodes[0].complete).into(),
                    ratio: if total == 0 {
                        0.
                    } else {
                        (*bytes as f64 / total as f64) as f32
                    },
                    tint: tint(i),
                })
                .collect::<Vec<_>>(),
        ))
        .into(),
    );
    let summary = format!(
        "逻辑占用 {} / {} 个文件 / {} 项未读取{}",
        bytes_label(Some(total), analysis.nodes[0].complete),
        analysis.files,
        analysis.skipped,
        if analysis.cancelled {
            "；已取消，结果不完整"
        } else {
            ""
        }
    );
    ui.set_space_summary(i18n::translate(&summary, english).into());
    ui.set_space_path(
        node.path
            .as_ref()
            .map(|p| display_path(p))
            .unwrap_or_else(|| i18n::translate("全部来源", english))
            .into(),
    );
    ui.set_space_can_up(!state.space_parents.is_empty());
    let advice = format!(
        "存档占用：{}。先备份，再检查不再使用的世界。\n模组、包与光影占用：{}。重复候选只说明检测时字节相同，请核对实例依赖。\n其他文件占用：{}。可双击逐层查看日志、截图和下载缓存；本程序不会自动判断可删除。\n分析是当前快照，文件变化后请重新分析。未读取项使大小显示为下限；合并矩形可通过上一级或目录钻取查看。",
        bytes_label(Some(analysis.categories[0]), true),
        bytes_label(
            Some(
                analysis.categories[1]
                    + analysis.categories[2]
                    + analysis.categories[3]
                    + analysis.categories[4]
            ),
            true
        ),
        bytes_label(Some(analysis.categories[5]), true)
    );
    ui.set_space_advice(i18n::translate(&advice, english).into());
}

// 范围按真实路径集合比较；重复实例路径或显示文字变化不应清空已完成快照。
fn scope_keys(paths: &[PathBuf]) -> Vec<String> {
    let mut keys = paths.iter().map(|p| path_key(p)).collect::<Vec<_>>();
    keys.sort_by_key(|key| (key.len(), key.clone()));
    let mut roots = Vec::<String>::new();
    for key in keys {
        if !roots.iter().any(|root| {
            key == *root
                || key.starts_with(&format!(
                    "{}{}",
                    root.trim_end_matches(std::path::MAIN_SEPARATOR),
                    std::path::MAIN_SEPARATOR
                ))
        }) {
            roots.push(key);
        }
    }
    roots
}

fn update_space_scopes(ui: &AppWindow, state: &mut UiState) {
    let all = state
        .settings
        .sources
        .iter()
        .map(|s| s.path.clone())
        .chain(state.games.iter().map(|g| g.path.clone()))
        .collect::<Vec<_>>();
    let mut scopes = vec![all];
    let mut labels = vec![i18n::translate("全部来源", ui.get_english())];
    for source in &state.settings.sources {
        scopes.push(vec![source.path.clone()]);
        labels.push(display_path(&source.path));
    }
    for game in &state.games {
        if !scopes.iter().skip(1).any(|p| {
            p.first()
                .is_some_and(|p| path_key(p) == path_key(&game.path))
        }) {
            scopes.push(vec![game.path.clone()]);
            labels.push(format!("{} · {}", game.name, display_path(&game.path)));
        }
    }
    let current = ui.get_space_index().max(0) as usize;
    let previous = state
        .space_scopes
        .get(current)
        .map(|paths| scope_keys(paths));
    let next = scopes.get(current).map(|paths| scope_keys(paths));
    if previous != next {
        state.analysis = None;
        state.space_parents.clear();
        ui.set_space_tiles(ModelRc::default());
        ui.set_space_bars(ModelRc::default());
        ui.set_space_can_up(false);
        ui.set_space_can_view(false);
        ui.set_space_can_reveal(false);
        ui.set_space_summary(
            i18n::translate(
                if state.auto_analysis_request.is_some() {
                    "索引刷新后自动分析新增目录…"
                } else {
                    "请开始分析，统计实际文件大小"
                },
                ui.get_english(),
            )
            .into(),
        );
    }
    state.space_scopes = scopes;
    ui.set_space_scopes(strings(labels));
    if ui.get_space_index() as usize >= state.space_scopes.len() {
        ui.set_space_index(0);
    }
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

fn operation_signature(ui: &AppWindow, ids: &[String]) -> String {
    serde_json::to_string(&(
        ids,
        ui.get_operation_action(),
        ui.get_operation_name().to_string(),
        ui.get_operation_parent().to_string(),
        ui.get_operation_backup().to_string(),
        ui.get_operation_conflict(),
        ui.get_operation_version_scope(),
    ))
    .unwrap_or_default()
}

pub fn run() -> Result<()> {
    let db = storage_path()?;
    Index::open(&db)?;
    let ui = AppWindow::new()?;
    ui.global::<I18n>()
        .on_translate(|value| i18n::translate(&value, true).into());
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
    let callbacks = sender.clone();
    let closing = Arc::new(AtomicBool::new(false));
    let worker_closing = closing.clone();
    std::thread::spawn(move || {
        worker(
            db,
            commands,
            events,
            worker_cancel,
            callbacks,
            worker_closing,
        )
    });
    let state = Rc::new(RefCell::new(UiState::default()));
    state.borrow_mut().diagnostics.push("阶段 3：安全文件管理。PCL2 / PCL CE / HMCL 当前仅具备 OpenLauncher；指定实例直接启动尚未验证。\n大小口径：逻辑字节；内容与世界、游戏目录大小不可叠加。\n文件操作先预览再确认，不覆盖；移动先复制校验，再将原件移入回收站。\n收藏与标签只写入本程序数据库；重命名和移动后同步注释。\n数据包必须包含根级 pack.mcmeta 和 data/；不跟随链接、junction 或重解析点。".into());
    let weak = ui.as_weak();
    let appearance_sender = sender.clone();
    let appearance_ui = weak.clone();
    let appearance_state = state.clone();
    ui.on_appearance_changed(move || {
        if let Some(ui) = appearance_ui.upgrade() {
            let mut state = appearance_state.borrow_mut();
            state.appearance_request += 1;
            state.appearance_pending = true;
            let mut settings = state.settings.clone();
            settings.english = ui.get_english();
            settings.light_theme = ui.get_light_theme();
            apply_settings(&ui, &mut state, settings);
            state.query_due = Some(Instant::now());
            let _ = appearance_sender.send(Command::Appearance(
                ui.get_english(),
                ui.get_light_theme(),
                state.appearance_request,
            ));
        }
    });
    let analyze_state = state.clone();
    let analyze_ui = weak.clone();
    let analyze_sender = sender.clone();
    ui.on_analyze_space(move || {
        if let Some(ui) = analyze_ui.upgrade() {
            let mut state = analyze_state.borrow_mut();
            if ui.get_scanning() {
                return;
            }
            let paths = state
                .space_scopes
                .get(ui.get_space_index().max(0) as usize)
                .cloned()
                .unwrap_or_default();
            if paths.is_empty() {
                return;
            }
            state.analysis_request += 1;
            state.space_node = 0;
            state.space_parents.clear();
            ui.set_scanning(true);
            let _ = analyze_sender.send(Command::Analyze(paths, state.analysis_request));
            ui.set_space_summary(i18n::translate("正在分析实际目录…", ui.get_english()).into());
        }
    });
    let scope_state = state.clone();
    let scope_ui = weak.clone();
    ui.on_space_scope_changed(move || {
        if let Some(ui) = scope_ui.upgrade()
            && let Ok(mut state) = scope_state.try_borrow_mut()
        {
            if state.assigned_space_index.take() == Some(ui.get_space_index()) {
                return;
            }
            // 设置自动分析范围也会触发延迟的 changed 回调，不能把自己的请求作废。
            if state.auto_analysis_request == Some(state.analysis_request)
                && state.auto_analysis_path.as_ref().is_some_and(|path| {
                    state
                        .space_scopes
                        .get(ui.get_space_index().max(0) as usize)
                        .is_some_and(|scope| {
                            scope.len() == 1 && path_key(&scope[0]) == path_key(path)
                        })
                })
            {
                return;
            }
            state.auto_analysis_request = None;
            state.auto_analysis_path = None;
            state.analysis_request += 1;
            state.analysis = None;
            state.space_parents.clear();
            state.space_node = 0;
            ui.set_space_tiles(ModelRc::default());
            ui.set_space_bars(ModelRc::default());
            ui.set_space_selected(-1);
            ui.set_space_can_up(false);
            ui.set_space_can_reveal(false);
            ui.set_space_can_view(false);
            ui.set_space_info("".into());
            ui.set_space_summary(
                i18n::translate("请开始分析，统计实际文件大小", ui.get_english()).into(),
            );
        }
    });
    let space_state = state.clone();
    let space_ui = weak.clone();
    let space_sender = sender.clone();
    ui.on_space_action(move |node, action| {
        let Some(ui) = space_ui.upgrade() else {
            return;
        };
        let mut state = space_state.borrow_mut();
        if action == 2 {
            if let Some(parent) = state.space_parents.pop() {
                state.space_node = parent;
                ui.set_space_selected(-1);
                ui.set_space_can_reveal(false);
                ui.set_space_can_view(false);
                render_space(&ui, &state);
            }
            return;
        }
        let Some(entry) = state
            .analysis
            .as_ref()
            .and_then(|a| a.nodes.get(node as usize))
            .cloned()
        else {
            ui.set_space_selected(-1);
            ui.set_space_can_reveal(false);
            ui.set_space_can_view(false);
            return;
        };
        if action == 1 && entry.directory && !entry.children.is_empty() {
            let parent = state.space_node;
            state.space_parents.push(parent);
            state.space_node = node as usize;
            ui.set_space_selected(-1);
            ui.set_space_can_reveal(false);
            ui.set_space_can_view(false);
            render_space(&ui, &state);
        } else if action == 3 {
            if let Some(path) = entry.path {
                let _ = space_sender.send(Command::RevealPath(path));
            }
        } else if action == 4 {
            if let Some(id) = entry.item_id {
                drop(state);
                ui.set_filter_index(0);
                ui.set_source_index(0);
                ui.set_favorite_only(false);
                ui.set_tag_filter("".into());
                ui.set_search(
                    entry
                        .path
                        .map(|p| display_path(&p))
                        .unwrap_or_default()
                        .into(),
                );
                ui.set_page(0);
                ui.invoke_navigate(0);
                ui.set_selected_id(id.clone().into());
                let _ = space_sender.send(Command::Detail(id));
            }
        } else {
            ui.set_space_selected(node);
            ui.set_space_can_reveal(entry.path.is_some());
            ui.set_space_can_view(entry.item_id.is_some());
            ui.set_space_info(
                format!(
                    "{} · {}\n{}\n{}",
                    entry.name,
                    bytes_label(Some(entry.bytes), entry.complete),
                    entry.path.map(|p| display_path(&p)).unwrap_or_default(),
                    entry.owner
                )
                .into(),
            );
        }
    });
    let action_ui = weak.clone();
    ui.on_entry_action(move |id, action| {
        let Some(ui) = action_ui.upgrade() else {
            return;
        };
        ui.set_selected_id(id.clone());
        ui.invoke_selected(id);
        match action {
            0 => ui.invoke_reveal_selected(),
            1 => ui.invoke_copy_selected(),
            2..=6 => ui.invoke_begin_operation(action - 2, true),
            7 => {
                ui.set_detail_favorite(!ui.get_detail_favorite());
                ui.invoke_annotate_selected();
            }
            8 if ui.get_can_open() => ui.invoke_open_launcher(),
            _ => (),
        }
    });
    let key_ui = weak.clone();
    ui.on_keyboard_action(move |action| {
        if let Some(ui) = key_ui.upgrade() {
            match action {
                10 => ui.invoke_select_all(),
                11 => ui.invoke_clear_selection(),
                12 | 13 => {
                    let entries = ui.get_entries();
                    let current = entries
                        .iter()
                        .position(|row| row.id == ui.get_selected_id());
                    let next = if action == 12 {
                        current.unwrap_or(0).saturating_sub(1)
                    } else {
                        current
                            .map_or(0, |i| i + 1)
                            .min(entries.row_count().saturating_sub(1))
                    };
                    if let Some(row) = entries.row_data(next) {
                        ui.set_selected_id(row.id.clone());
                        ui.invoke_selected(row.id);
                    }
                }
                _ => {
                    if !ui.get_selected_id().is_empty() {
                        ui.invoke_entry_action(ui.get_selected_id(), action);
                    }
                }
            }
        }
    });
    let close_ui = weak.clone();
    let close_state = state.clone();
    let close_cancel = cancel.clone();
    ui.window().on_close_requested(move || {
        closing.store(true, Ordering::SeqCst);
        close_cancel.store(true, Ordering::SeqCst);
        if let Some(ui) = close_ui.upgrade()
            && ui.get_scanning()
        {
            close_state.borrow_mut().close_requested = true;
            ui.set_status(
                i18n::translate("正在取消任务，完成收尾后关闭…", ui.get_english()).into(),
            );
            slint::CloseRequestResponse::KeepWindowShown
        } else {
            slint::CloseRequestResponse::HideWindow
        }
    });
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
            ui.set_current_row_index(-1);
            nav_state.borrow_mut().selected = None;
            nav_state.borrow_mut().checked.clear();
            ui.set_checked_count(0);
            if ui.get_page() == 7 {
                let _ = nav_sender.send(Command::History);
            }
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
            ui.set_status(
                i18n::translate(
                    "正在取消未完成部分；已提交的文件操作保留…",
                    ui.get_english(),
                )
                .into(),
            );
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
            ui.set_current_row_index(
                state
                    .row_positions
                    .get(id.as_str())
                    .map_or(-1, |i| *i as i32),
            );
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
    let script_ui = weak.clone();
    let script_state = state.clone();
    let script_sender = sender.clone();
    ui.on_launch_script(move || {
        if let Some(ui) = script_ui.upgrade() {
            let state = script_state.borrow();
            if ui.get_can_launch_script()
                && !ui.get_scanning()
                && let Some(l) = selected_launcher(&ui, &state)
            {
                let _ = script_sender.send(Command::LaunchScript(
                    ui.get_selected_id().to_string(),
                    l.id.clone(),
                ));
            }
        }
    });
    let target_ui = weak.clone();
    let target_state = state.clone();
    let target_discovery = sender.clone();
    ui.on_mapping_target_changed(move || {
        if let Some(ui) = target_ui.upgrade() {
            ui.set_mapping_script("".into());
            ui.set_mapping_script_workdir("".into());
            if let Ok(mut state) = target_state.try_borrow_mut() {
                queue_script_discovery(&ui, &mut state, &target_discovery);
            }
        }
    });
    let begin_discovery = sender.clone();
    ui.on_begin_mapping(move || {
        if let Some(ui) = map_ui.upgrade() {
            let mut state = begin_map_state.borrow_mut();
            ui.set_mapping_script("".into());
            ui.set_mapping_script_workdir("".into());
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
                if let Some(installation) = selected_launcher(&ui, &state)
                    && let Some(mapping) = resolve_mapping(&state.settings, item, &installation.id)
                {
                    if let Some(position) = state
                        .versions
                        .iter()
                        .position(|v| v.path == mapping.version_path)
                    {
                        ui.set_version_index(position as i32);
                    }
                    if let Some(script) = &mapping.script {
                        ui.set_mapping_script(display_path(&script.path).into());
                        ui.set_mapping_script_workdir(display_path(&script.working_dir).into());
                    }
                }
            }
            ui.set_mapping_visible(true);
            if ui.get_mapping_script().is_empty() {
                queue_script_discovery(&ui, &mut state, &begin_discovery);
            } else {
                ui.set_mapping_script_notice(
                    i18n::translate(
                        "已有脚本关联；自动识别不会覆盖手动填写的脚本。",
                        ui.get_english(),
                    )
                    .into(),
                );
            }
        }
    });
    let discovery_ui = weak.clone();
    let discovery_state = state.clone();
    let discovery_sender = sender.clone();
    ui.on_discover_script(move || {
        if let Some(ui) = discovery_ui.upgrade() {
            queue_script_discovery(&ui, &mut discovery_state.borrow_mut(), &discovery_sender);
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
                    if ui.get_mapping_script().trim().is_empty() {
                        None
                    } else {
                        let path = PathBuf::from(ui.get_mapping_script().trim());
                        let working_dir = if ui.get_mapping_script_workdir().trim().is_empty() {
                            path.parent().unwrap_or(&path).to_path_buf()
                        } else {
                            PathBuf::from(ui.get_mapping_script_workdir().trim())
                        };
                        Some(LaunchScript { path, working_dir })
                    },
                ));
            } else {
                ui.set_status(
                    i18n::translate("请先添加启动器并扫描可用版本定义", ui.get_english()).into(),
                );
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
                ui.set_status(
                    i18n::translate("请先添加 Minecraft 根目录", ui.get_english()).into(),
                );
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

    let toggle_ui = weak.clone();
    let toggle_state = state.clone();
    ui.on_toggle_entry(move |id| {
        if let Some(ui) = toggle_ui.upgrade() {
            let mut state = toggle_state.borrow_mut();
            if !state.checked.remove(id.as_str()) {
                state.checked.insert(id.to_string());
            }
            if let Some(&row) = state.row_positions.get(id.as_str())
                && let Some(mut data) = state.row_model.row_data(row)
            {
                data.checked = state.checked.contains(id.as_str());
                state.row_model.set_row_data(row, data);
            }
            ui.set_checked_count(state.checked.len() as i32);
        }
    });
    let all_ui = weak.clone();
    let all_state = state.clone();
    ui.on_select_all(move || {
        if let Some(ui) = all_ui.upgrade() {
            let mut state = all_state.borrow_mut();
            state.checked = state
                .items
                .iter()
                .filter(|i| i.kind != ItemKind::Instance && i.available)
                .take(256)
                .map(|i| i.id.clone())
                .collect();
            for n in 0..state.row_model.row_count() {
                if let Some(mut row) = state.row_model.row_data(n) {
                    row.checked = state.checked.contains(row.id.as_str());
                    state.row_model.set_row_data(n, row);
                }
            }
            ui.set_checked_count(state.checked.len() as i32);
        }
    });
    let clear_ui = weak.clone();
    let clear_state = state.clone();
    ui.on_clear_selection(move || {
        if let Some(ui) = clear_ui.upgrade() {
            let mut state = clear_state.borrow_mut();
            state.checked.clear();
            for n in 0..state.row_model.row_count() {
                if let Some(mut row) = state.row_model.row_data(n) {
                    row.checked = false;
                    state.row_model.set_row_data(n, row);
                }
            }
            ui.set_checked_count(0);
        }
    });
    let begin_ui = weak.clone();
    let begin_state = state.clone();
    ui.on_begin_operation(move |action, only_selected| {
        if let Some(ui) = begin_ui.upgrade() {
            if ui.get_scanning() {
                return;
            }
            let ids = {
                let state = begin_state.borrow();
                if action == 5 {
                    Vec::new()
                } else if only_selected || state.checked.is_empty() {
                    vec![ui.get_selected_id().to_string()]
                } else {
                    state
                        .items
                        .iter()
                        .filter(|i| state.checked.contains(&i.id))
                        .map(|i| i.id.clone())
                        .collect()
                }
            };
            if action != 5 && (ids.is_empty() || ids.iter().any(String::is_empty)) {
                ui.set_status(i18n::translate("请先选中或勾选条目", ui.get_english()).into());
                return;
            }
            ui.set_operation_visible(false);
            ui.set_operation_action(action);
            ui.set_operation_ready(false);
            ui.set_operation_confirmed(false);
            ui.set_operation_parent("".into());
            ui.set_operation_backup("".into());
            ui.set_operation_conflict(0);
            ui.set_operation_version_scope(0);
            let name = {
                let state = begin_state.borrow();
                ids.first()
                    .and_then(|id| state.items.iter().find(|i| &i.id == id))
                    .map(|i| i.folder.clone())
                    .unwrap_or_default()
            };
            ui.set_operation_name(name.into());
            ui.set_operation_preview(
                i18n::translate(
                    "填写操作参数后生成预览；此步骤只读取源文件。",
                    ui.get_english(),
                )
                .into(),
            );
            let mut state = begin_state.borrow_mut();
            state.operation_ids = ids;
            state.operation_plan = None;
            ui.set_operation_visible(true);
        }
    });
    let edited_ui = weak.clone();
    let edited_state = state.clone();
    ui.on_operation_edited(move || {
        if let Some(ui) = edited_ui.upgrade() {
            ui.set_operation_ready(false);
            ui.set_operation_confirmed(false);
            if let Ok(mut state) = edited_state.try_borrow_mut() {
                state.operation_plan = None;
            }
            if ui.get_operation_visible() {
                ui.set_operation_preview(
                    i18n::translate("参数已变化，请重新生成预览。", ui.get_english()).into(),
                );
            }
        }
    });
    let prepare_ui = weak.clone();
    let prepare_state = state.clone();
    let prepare_sender = sender.clone();
    ui.on_prepare_operation(move || {
        if let Some(ui) = prepare_ui.upgrade() {
            if ui.get_scanning() {
                return;
            }
            let action = match ui.get_operation_action() {
                0 => Action::Rename,
                1 => Action::Copy,
                2 => Action::Move,
                3 => Action::Backup,
                4 => Action::Recycle,
                _ => Action::Restore,
            };
            ui.set_operation_ready(false);
            ui.set_operation_confirmed(false);
            ui.set_operation_preview(
                i18n::translate("正在后台核对路径和文件哈希…", ui.get_english()).into(),
            );
            ui.set_scanning(true);
            let signature = {
                let mut state = prepare_state.borrow_mut();
                let signature = operation_signature(&ui, &state.operation_ids);
                state.operation_signature = signature.clone();
                signature
            };
            let _ = prepare_sender.send(Command::PrepareOperation(
                prepare_state.borrow().operation_ids.clone(),
                action,
                PathBuf::from(ui.get_operation_parent().as_str()),
                ui.get_operation_name().to_string(),
                if ui.get_operation_conflict() == 0 {
                    Conflict::Abort
                } else {
                    Conflict::KeepBoth
                },
                if ui.get_operation_version_scope() == 0 {
                    VersionScope::CoreOnly
                } else {
                    VersionScope::IndependentDirectory
                },
                PathBuf::from(ui.get_operation_backup().as_str()),
                signature,
            ));
        }
    });
    let confirm_ui = weak.clone();
    let confirm_state = state.clone();
    let confirm_sender = sender.clone();
    ui.on_confirm_operation(move || {
        if let Some(ui) = confirm_ui.upgrade()
            && ui.get_operation_ready()
            && ui.get_operation_confirmed()
            && !ui.get_scanning()
        {
            let mut state = confirm_state.borrow_mut();
            if operation_signature(&ui, &state.operation_ids) != state.operation_signature {
                state.operation_plan = None;
                ui.set_operation_ready(false);
                ui.set_operation_confirmed(false);
                ui.set_operation_preview(
                    i18n::translate("参数已改变，请重新生成预览。", ui.get_english()).into(),
                );
                return;
            }
            let Some(plan) = state.operation_plan.take() else {
                return;
            };
            ui.set_operation_ready(false);
            ui.set_operation_confirmed(false);
            ui.set_operation_preview(
                i18n::translate("正在执行；可取消尚未提交的部分。", ui.get_english()).into(),
            );
            ui.set_scanning(true);
            let _ = confirm_sender.send(Command::ExecuteOperation(Box::new(plan)));
        }
    });
    let dismiss_ui = weak.clone();
    let dismiss_state = state.clone();
    let dismiss_cancel = cancel.clone();
    ui.on_dismiss_operation(move || {
        if let Some(ui) = dismiss_ui.upgrade() {
            if ui.get_scanning() {
                dismiss_cancel.store(true, Ordering::SeqCst);
            }
            ui.set_operation_visible(false);
            ui.set_operation_ready(false);
            ui.set_operation_confirmed(false);
            dismiss_state.borrow_mut().operation_plan = None;
        }
    });
    let browse_operation_sender = sender.clone();
    ui.on_browse_operation(move |field| {
        let _ = browse_operation_sender.send(Command::BrowseOperation(field));
    });
    let history_sender = sender.clone();
    ui.on_show_history(move || {
        let _ = history_sender.send(Command::History);
    });
    let reveal_history_sender = sender.clone();
    ui.on_reveal_history(move || {
        let _ = reveal_history_sender.send(Command::RevealHistory);
    });
    let license_sender = sender.clone();
    ui.on_export_licenses(move || {
        let _ = license_sender.send(Command::ExportLicenses);
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
                Event::Settings(mut settings) => {
                    if state.appearance_pending {
                        settings.english = state.settings.english;
                        settings.light_theme = state.settings.light_theme;
                    }
                    let first = state.last_query.is_none();
                    let previous_source=ui.get_source_index().checked_sub(1).and_then(|i|state.settings.sources.get(i as usize)).map(|s|path_key(&s.path));
                    apply_settings(&ui,&mut state,settings);
                    ui.set_source_index(previous_source.and_then(|key|state.settings.sources.iter().position(|s|path_key(&s.path)==key)).map(|i|i as i32+1).unwrap_or(0));
                    if first { queue_query(&ui,&mut state,&poll_sender); let _ = poll_sender.send(Command::Scan); }
                    else {state.query_due=Some(Instant::now());}
                }
                Event::AppearanceSaved(request) => {
                    if request == state.appearance_request { state.appearance_pending = false; }
                }
                Event::ScriptDiscovered(script, signature, notice, request) => {
                    if ui.get_mapping_visible() && request == state.script_request && script_signature(&ui, &state).as_ref() == Some(&signature) {
                        if (ui.get_mapping_script().to_string(), ui.get_mapping_script_workdir().to_string()) != state.script_initial {
                            ui.set_mapping_script_notice(i18n::translate("手动填写的脚本或工作目录已保留。", ui.get_english()).into());
                            continue;
                        }
                        if let Some(script) = script {
                            ui.set_mapping_script(display_path(&script.path).into());
                            ui.set_mapping_script_workdir(display_path(&script.working_dir).into());
                        }
                        ui.set_mapping_script_notice(i18n::translate(&notice, ui.get_english()).into());
                    }
                }
                Event::SourceAdded(path, request) => {
                    state.analysis_request = request;
                    state.auto_analysis_request = Some(request);
                    state.auto_analysis_path = Some(path.clone());
                    state.analysis = None;
                    state.space_node = 0;
                    state.space_parents.clear();
                    ui.set_page(6);
                    ui.set_selected_id("".into());
                    state.selected = None;
                    state.checked.clear();
                    ui.set_checked_count(0);
                    let index = state.space_scopes.iter().enumerate().skip(1).find(|(_,scope)| scope.len() == 1 && path_key(&scope[0]) == path_key(&path)).map(|(index,_)| index).unwrap_or(0) as i32;
                    if index != ui.get_space_index() { state.assigned_space_index = Some(index); }
                    ui.set_space_index(index);
                    ui.set_space_tiles(ModelRc::default());
                    ui.set_space_bars(ModelRc::default());
                    ui.set_space_bytes("".into());
                    ui.set_space_can_up(false);ui.set_space_can_reveal(false);ui.set_space_can_view(false);ui.set_space_info("".into());
                    ui.set_scanning(true);
                    ui.set_space_summary(i18n::translate("索引刷新后自动分析新增目录…", ui.get_english()).into());
                }
                Event::SpaceStatus(message,request) => {
                    if request == state.analysis_request {
                        if message.starts_with("自动空间分析已取消") || message.starts_with("空间分析失败") { state.auto_analysis_request = None; state.auto_analysis_path = None; }
                        ui.set_space_summary(i18n::translate(&message,ui.get_english()).into());
                        ui.set_status(i18n::translate(&message,ui.get_english()).into());
                    }
                }
                Event::Analysis(analysis,request) => {
                    if request==state.analysis_request {
                        state.auto_analysis_request = None;
                        state.auto_analysis_path = None;
                        state.space_node=0;state.space_parents.clear();
                        if analysis.nodes[0].children.len()==1 {state.space_node=analysis.nodes[0].children[0];state.space_parents.push(0);}
                        state.analysis=Some(analysis);ui.set_space_selected(-1);ui.set_space_can_reveal(false);ui.set_space_can_view(false);ui.set_space_info("".into());render_space(&ui,&state);
                    }
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
                    update_space_scopes(&ui,&mut state);
                    ui.set_filters(strings(std::iter::once(i18n::translate("全部游戏目录",ui.get_english())).chain(state.games.iter().map(|g|format!("{} · {}",g.name,display_path(&g.path)))).collect()));
                    ui.set_game_labels(strings(state.games.iter().map(|g| format!("{} · {}",g.ownership,display_path(&g.path))).collect()));
                    ui.set_version_labels(strings(state.versions.iter().map(|v| format!("{} · {}",v.name,display_path(&v.path))).collect()));
                    ui.set_counts(format!("{} 个来源 / {} 个版本 / {} 个游戏目录 / {} 张存档\n{} 个资源包 / {} 个模组 / {} 个光影包 / {} 个数据包",state.settings.sources.len(),stats.versions,stats.instances,stats.worlds,stats.resource_packs,stats.mods,stats.shader_packs,stats.data_packs).into());
                    ui.set_summary(format!("已索引世界逻辑大小：{}。包含缓存与可能的大小下限；世界和游戏目录大小不可相加。", bytes_label(Some(stats.world_bytes),false)).into());
                    ui.set_counts(i18n::translate(&ui.get_counts(),ui.get_english()).into());ui.set_summary(i18n::translate(&ui.get_summary(),ui.get_english()).into());
                    state.checked.retain(|id|items.iter().any(|i|&i.id==id));ui.set_checked_count(state.checked.len() as i32);
                    let rows=items.iter().map(|i| {let key=icon_key(i);EntryRow {
                        id: i.id.clone().into(), title: format!("{}{}",if i.favorite {"★ "}else {""},i.name).into(), kind: i18n::translate(i.kind.label(),ui.get_english()).into(), owner: i18n::translate(&i.ownership,ui.get_english()).into(),
                        path: display_path(&i.path).into(), size: i18n::translate(&bytes_label(i.size,i.size_complete),ui.get_english()).into(),
                        time: i18n::translate(&time_label(i.last_played_ms.or(i.modified_ms)),ui.get_english()).into(), available: i.available,
                        icon: poll_icons.borrow().loaded.get(&key).cloned().flatten().unwrap_or_default(), icon_key:key.into(),
                        checked:state.checked.contains(&i.id),
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
                Event::Status(message) => {ui.set_status(i18n::translate(&message,ui.get_english()).into());state.raw_status=message;},
                Event::Progress(progress,message) => { ui.set_progress(progress); ui.set_status(i18n::translate(&message,ui.get_english()).into());state.raw_status=message; }
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
                    3 => {
                        ui.set_mapping_script(display_path(&path).into());
                        if ui.get_mapping_script_workdir().is_empty() && let Some(parent) = path.parent() {
                            ui.set_mapping_script_workdir(display_path(parent).into());
                        }
                    }
                    4 => ui.set_mapping_script_workdir(display_path(&path).into()),
                    _ => ui.set_edit_java(display_path(&path).into()),
                },
                Event::EditorSaved => { ui.set_editor_visible(false); ui.set_mapping_visible(false); launch_info(&ui,&state); }
                Event::SourcesChanged => { let _ = poll_sender.send(Command::Scan); }
                Event::Duplicates(items, summary) => {state.duplicate_items=items;ui.set_duplicate_summary(i18n::translate(&summary,ui.get_english()).into());state.query_due=Some(Instant::now());}
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
                Event::OperationPrepared(plan, signature) => {if ui.get_operation_visible() && operation_signature(&ui,&state.operation_ids)==signature {ui.set_operation_preview(plan.preview_localized(ui.get_english()).into());ui.set_operation_ready(true);state.operation_plan=Some(*plan);ui.set_status(i18n::translate("预览已生成；核对真实路径并勾选确认后才能执行",ui.get_english()).into());}}
                Event::OperationError(message) => {ui.set_operation_ready(false);state.operation_plan=None;ui.set_operation_preview(i18n::translate(&message,ui.get_english()).into());ui.set_status(i18n::translate(&message,ui.get_english()).into());state.diagnostics.push(message);}
                Event::OperationHistory(history) => {ui.set_operation_history(i18n::translate(&history,ui.get_english()).into());state.raw_history=history;},
                Event::OperationBrowse(field,path) => {if field==1{ui.set_operation_backup(display_path(&path).into());}else{ui.set_operation_parent(display_path(&path).into());}}
                Event::OperationFinished => {ui.set_operation_visible(false);ui.set_operation_ready(false);state.operation_plan=None;state.query_due=Some(Instant::now());},
            }
        }
        { let mut cache=poll_icons.borrow_mut(); while let Some(request)=cache.deferred.pop_front() {
            match icon_sender.try_send(request) {Ok(())=>(),Err(mpsc::TrySendError::Full(request))=>{cache.deferred.push_front(request);break;},Err(mpsc::TrySendError::Disconnected(request))=>{cache.pending.remove(&request.key);} }
        } }
        ui.set_diagnostics(state.diagnostics.iter().map(|d|i18n::translate(d,ui.get_english())).collect::<Vec<_>>().join("\n\n").into());
        if state.close_requested && !ui.get_scanning(){let _=slint::quit_event_loop();return;}
        if state.query_due.is_some_and(|due| Instant::now() >= due) { queue_query(&ui,&mut state,&poll_sender); }
    });
    #[cfg(feature = "ui-smoke")]
    let _smoke_timer = crate::ui_smoke::start(&ui);
    #[cfg(feature = "ui-smoke")]
    let _stage2_smoke_timer = crate::ui_stage2_smoke::start(&ui);
    #[cfg(feature = "ui-smoke")]
    let _stage3_smoke_timer = crate::ui_stage3_smoke::start(&ui);
    #[cfg(feature = "ui-smoke")]
    let _stage4_smoke_timer = crate::ui_stage4_smoke::start(&ui);
    ui.run()?;
    cancel.store(true, Ordering::SeqCst);
    timer.stop();
    Ok(())
}
