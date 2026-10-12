//! 需要确认且受预算约束的文件操作；不永久删除源文件。
//! 在目标旁暂存副本，校验后以不覆盖方式提交。
use crate::{models::*, platform::*, recycle};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

const MAX_ENTRIES: usize = 100_000;
const RECORD_LIMIT: u64 = 32 * 1024 * 1024;
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    Rename,
    Copy,
    Move,
    Backup,
    Recycle,
    Restore,
}
impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::Rename => "重命名",
            Self::Copy => "复制",
            Self::Move => "移动",
            Self::Backup => "备份",
            Self::Recycle => "移入回收站",
            Self::Restore => "恢复备份",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conflict {
    Abort,
    KeepBoth,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VersionScope {
    CoreOnly,
    IndependentDirectory,
}

#[derive(Clone, Debug)]
pub struct Policy {
    roots: Vec<PathBuf>,
    games: Vec<PathBuf>,
    protected: Vec<PathBuf>,
    pub records: PathBuf,
}
impl Policy {
    pub fn new(settings: &Settings, games: &[Item], data: &Path) -> Result<Self> {
        let data = checked_path(data)?;
        let mut roots = Vec::new();
        let mut game_dirs = Vec::new();
        for source in &settings.sources {
            if source.kind == SourceKind::Root {
                roots.push(source.path.clone());
            } else {
                game_dirs.push(source.path.clone());
            }
            if let Some(root) = &source.associated_root {
                roots.push(root.clone());
            }
        }
        game_dirs.extend(games.iter().filter_map(|g| g.game_dir.clone()));
        let mut protected = vec![data.clone()];
        if let Some(home) = std::env::var_os("USERPROFILE") {
            protected.push(PathBuf::from(home));
        }
        protected.extend(settings.launchers.iter().map(|l| l.executable.clone()));
        Ok(Self {
            roots,
            games: game_dirs,
            protected,
            records: data.join("operations"),
        })
    }
    fn guard(&self, path: &Path, allow_game: bool) -> Result<PathBuf> {
        let path = checked_path(path)?;
        if path.parent().is_none() || path.components().count() <= 2 {
            bail!("禁止操作磁盘根目录：{}", display_path(&path));
        }
        let contains = |p: &Path| {
            let own = path_key(&path);
            let other = path_key(p);
            other == own || other.starts_with(&(own + "\\"))
        };
        for p in &self.protected {
            if contains(p) {
                bail!("禁止操作受保护路径：{}", display_path(&path));
            }
        }
        for root in &self.roots {
            if contains(root) || contains(&root.join("versions")) {
                bail!("禁止操作 Minecraft 根目录或 versions 容器");
            }
        }
        for game in &self.games {
            if contains(game) && !(allow_game && path_key(game) == path_key(&path)) {
                bail!("禁止操作整个共享或自定义游戏目录");
            }
            for name in [
                "saves",
                "mods",
                "resourcepacks",
                "shaderpacks",
                "screenshots",
            ] {
                if contains(&game.join(name)) && !allow_game {
                    bail!("禁止操作内容容器目录");
                }
            }
        }
        Ok(path)
    }
    fn game_for(&self, item: &Item) -> Result<PathBuf> {
        let game = checked_path(
            item.game_dir
                .as_ref()
                .context("条目没有可靠的实际游戏目录")?,
        )?;
        if !self.games.iter().any(|p| path_key(p) == path_key(&game)) {
            bail!("所属游戏目录已移除，请刷新");
        }
        Ok(game)
    }
    fn leaf(&self, item: &Item) -> Result<PathBuf> {
        if !item.available {
            bail!("离线条目不能执行文件操作");
        }
        let game = self.game_for(item)?;
        let path = self.guard(&item.path, false)?;
        let parent = path.parent().context("条目缺少父目录")?;
        let expected = match item.kind {
            ItemKind::World => game.join("saves"),
            ItemKind::ResourcePack => game.join("resourcepacks"),
            ItemKind::Mod => game.join("mods"),
            ItemKind::ShaderPack => game.join("shaderpacks"),
            ItemKind::DataPack => {
                let world = parent.parent().context("数据包没有所属存档")?;
                if path_key(world.parent().context("存档缺少父目录")?)
                    != path_key(&game.join("saves"))
                {
                    bail!("数据包不位于所选游戏目录的存档内");
                }
                world.join("datapacks")
            }
            _ => bail!("整个游戏目录不能直接执行此操作；版本定义只支持明确范围的回收站删除"),
        };
        if path_key(parent) != path_key(&expected) {
            bail!("条目物理路径与所属内容目录不一致，请刷新");
        }
        Ok(path)
    }
    fn target_parent(&self, kind: &ItemKind, parent: &Path, backup: bool) -> Result<PathBuf> {
        let parent = checked_path(parent)?;
        if !parent.is_dir() {
            bail!("目标必须是已有目录");
        }
        if backup {
            if self
                .roots
                .iter()
                .chain(&self.games)
                .any(|root| is_under(&parent, root))
            {
                bail!("备份必须保存在已添加游戏目录之外");
            }
        } else {
            let valid = self.games.iter().any(|game| {
                let expected = match kind {
                    ItemKind::World => game.join("saves"),
                    ItemKind::ResourcePack => game.join("resourcepacks"),
                    ItemKind::Mod => game.join("mods"),
                    ItemKind::ShaderPack => game.join("shaderpacks"),
                    ItemKind::DataPack => {
                        let Some(world) = parent.parent() else {
                            return false;
                        };
                        if world
                            .parent()
                            .is_none_or(|p| path_key(p) != path_key(&game.join("saves")))
                        {
                            return false;
                        }
                        world.join("datapacks")
                    }
                    _ => return false,
                };
                path_key(&expected) == path_key(&parent)
            });
            if !valid {
                bail!(
                    "目标应为已添加游戏目录内对应的 saves/resourcepacks/mods/shaderpacks，数据包目标应为某存档的 datapacks"
                );
            }
        }
        if self.protected.iter().any(|p| {
            path_key(p) == path_key(&parent)
                && path_key(p) != path_key(self.records.parent().unwrap())
        }) {
            bail!("目标目录受保护");
        }
        Ok(parent)
    }
}
fn is_under(path: &Path, parent: &Path) -> bool {
    let a = path_key(path);
    let b = path_key(parent);
    a == b || a.starts_with(&(b + "\\"))
}
pub fn valid_name(name: &str) -> Result<()> {
    if name.starts_with(".minecraft-explorer-")
        || name.is_empty()
        || name.chars().count() > 200
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
        || matches!(name, "." | "..")
    {
        bail!("名称必须是单个有效文件名，不可包含路径、控制字符或尾随空格/点");
    }
    let stem = name.split('.').next().unwrap_or("").to_uppercase();
    if ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].contains(&stem.as_str())
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
    {
        bail!("不能使用 Windows 保留设备名");
    }
    Ok(())
}
fn cancelled(flag: &AtomicBool) -> Result<()> {
    if flag.load(Ordering::SeqCst) {
        bail!("操作已取消，已提交的条目保留，未提交条目的原件保留");
    }
    Ok(())
}
fn unique_id() -> String {
    format!(
        "{}-{}-{}",
        now_ms(),
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::SeqCst)
    )
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub relative: PathBuf,
    pub directory: bool,
    pub size: u64,
    pub modified_ms: Option<i64>,
    pub hash: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Unit {
    pub item_id: String,
    pub kind: ItemKind,
    pub source: PathBuf,
    pub destination: Option<PathBuf>,
    pub entries: Vec<Entry>,
    pub whole_instance: bool,
    pub game_dir: Option<PathBuf>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    pub action: Action,
    pub units: Vec<Unit>,
}
impl Plan {
    fn check_budget(&self) -> Result<()> {
        if self.units.iter().map(|u| u.entries.len()).sum::<usize>() > MAX_ENTRIES {
            bail!("整个操作超过 100000 个文件/目录，请分批处理");
        }
        if serde_json::to_vec(self)?.len() as u64 > RECORD_LIMIT / 4 {
            bail!("操作清单超过 8 MiB，请分批处理");
        }
        Ok(())
    }
    pub fn preview(&self) -> String {
        self.preview_localized(false)
    }
    pub fn preview_localized(&self, english: bool) -> String {
        let tr = |value: &str| crate::i18n::translate(value, english);
        let mut text = format!(
            "{}：{} 个条目\n默认不覆盖。提交前再次核对源文件；批量操作逐项提交，可取消未提交部分。\n",
            tr(self.action.label()),
            self.units.len()
        );
        text = tr(&text);
        for unit in &self.units {
            let files = unit.entries.iter().filter(|e| !e.directory).count();
            let bytes = unit.entries.iter().map(|e| e.size).sum();
            text.push_str(&tr(&format!(
                "\n{}\n源：{}\n目标：{}\n{} 个文件 · {}\n",
                tr(unit.kind.label()),
                display_path(&unit.source),
                unit.destination
                    .as_ref()
                    .map(|p| display_path(p))
                    .unwrap_or_else(|| tr("Windows 回收站（不可用则中止）")),
                files,
                bytes_label(Some(bytes), true)
            )));
        }
        if self.action == Action::Move {
            text.push_str(&tr(
                "\n移动在目标副本校验成功后，将原件移入回收站；回收失败时保留两份并报告。\n",
            ));
        }
        if self.action == Action::Backup {
            text.push_str(&tr("\n目录备份保留内容和校验清单；可通过本程序恢复。session.lock 属于运行锁，不复制。\n"));
        }
        if self.units.iter().any(|u| u.whole_instance) {
            text.push_str(&tr(
                "\n注意：包含整个独立实例目录及其中所有存档、模组、配置。共享目录不会处理。\n",
            ));
        }
        text
    }
}

#[allow(clippy::too_many_arguments)]
pub fn prepare(
    policy: &Policy,
    items: &[Item],
    action: Action,
    parent: Option<&Path>,
    name: &str,
    conflict: Conflict,
    scope: VersionScope,
    cancel: &AtomicBool,
) -> Result<Plan> {
    cancelled(cancel)?;
    if items.is_empty() || items.len() > 256 {
        bail!("请选择 1–256 个条目");
    }
    if action == Action::Rename && items.len() != 1 {
        bail!("重命名请一次选择一个条目");
    }
    let id = unique_id();
    let mut units = Vec::new();
    let mut destinations = HashSet::new();
    let mut planned_entries = 0;
    for item in items {
        cancelled(cancel)?;
        let mut sources = Vec::new();
        let mut whole = false;
        if item.kind == ItemKind::Version {
            if action != Action::Recycle {
                bail!("版本定义仅支持按明确范围移入回收站，其他操作请选具体存档或内容");
            }
            let path = policy.guard(&item.path, true)?;
            if !policy.roots.iter().any(|r| {
                path.parent()
                    .is_some_and(|p| path_key(p) == path_key(&r.join("versions")))
            }) {
                bail!("版本目录不在已添加根目录的 versions 内");
            }
            match scope {
                VersionScope::CoreOnly => {
                    let folder = path
                        .file_name()
                        .context("版本目录缺少名称")?
                        .to_string_lossy();
                    let preferred = path.join(format!("{folder}.json"));
                    let mut jsons = Vec::new();
                    for entry in fs::read_dir(&path)? {
                        let entry = entry?;
                        let child = entry.path();
                        let meta = fs::symlink_metadata(&child)?;
                        if meta.is_file()
                            && !is_link(&meta)
                            && child
                                .extension()
                                .is_some_and(|e| e.eq_ignore_ascii_case("json"))
                        {
                            jsons.push(child);
                        }
                    }
                    let definition = if jsons.contains(&preferred) {
                        preferred
                    } else if jsons.len() == 1
                        && item.version_id.is_some()
                        && crate::minecraft::version_metadata(&jsons[0]).id == item.version_id
                    {
                        jsons.remove(0)
                    } else {
                        bail!(
                            "无法唯一确认版本核心 JSON；未处理其他配置文件，请选择具体内容或已确认的独立实例范围"
                        );
                    };
                    let jar = definition.with_extension("jar");
                    sources.push(definition);
                    if let Ok(meta) = fs::symlink_metadata(&jar) {
                        if is_link(&meta) {
                            bail!("核心 JAR 是链接，拒绝处理");
                        }
                        if meta.is_file() {
                            sources.push(jar);
                        }
                    }
                }
                VersionScope::IndependentDirectory => {
                    if item
                        .game_dir
                        .as_ref()
                        .is_none_or(|p| path_key(p) != path_key(&path))
                        || policy.roots.iter().any(|r| path_key(r) == path_key(&path))
                    {
                        bail!("未确认独立实例目录，禁止连带处理游戏数据");
                    }
                    whole = true;
                    sources.push(path);
                }
            }
        } else {
            sources.push(policy.leaf(item)?);
        }
        for source in sources {
            let entries = snapshot(&source, cancel)?;
            planned_entries += entries.len();
            if planned_entries > MAX_ENTRIES {
                bail!("整个操作超过 100000 个文件/目录，请分批处理");
            }
            let destination = if action == Action::Recycle {
                None
            } else {
                let (parent, target_name) = if action == Action::Rename {
                    valid_name(name)?;
                    // 保留归档或模组后缀，使条目仍可被识别。
                    if source.is_file() && source.extension() != Path::new(name).extension() {
                        bail!("文件重命名应保留原扩展名");
                    }
                    if source.is_file()
                        && source.file_name().is_some_and(|n| {
                            n.to_string_lossy()
                                .to_lowercase()
                                .ends_with(".jar.disabled")
                        })
                        && !name.to_lowercase().ends_with(".jar.disabled")
                    {
                        bail!("已禁用模组重命名应保留 .jar.disabled 后缀");
                    }
                    (
                        checked_path(source.parent().context("源缺少父目录")?)?,
                        name.to_string(),
                    )
                } else {
                    let parent = policy.target_parent(
                        &item.kind,
                        parent.context("请选择目标父目录")?,
                        action == Action::Backup,
                    )?;
                    let original = source
                        .file_name()
                        .context("源缺少名称")?
                        .to_string_lossy()
                        .into_owned();
                    let target_name = if action == Action::Backup {
                        format!("{original}.backup-{id}")
                    } else {
                        original
                    };
                    (parent, target_name)
                };
                if is_under(&parent, &source) {
                    bail!("目标不能位于源目录内部");
                }
                let target = destination(&parent, &target_name, conflict, &mut destinations)?;
                if path_key(&target) == path_key(&source) {
                    bail!("源和目标相同");
                }
                Some(target)
            };
            units.push(Unit {
                item_id: item.id.clone(),
                kind: item.kind.clone(),
                source,
                destination,
                entries,
                whole_instance: whole,
                game_dir: item.game_dir.clone(),
            });
        }
    }
    for (n, a) in units.iter().enumerate() {
        for b in &units[n + 1..] {
            if is_under(&a.source, &b.source) || is_under(&b.source, &a.source) {
                bail!("所选项目包含重复或父子路径，请分别操作");
            }
        }
    }
    let plan = Plan { id, action, units };
    plan.check_budget()?;
    Ok(plan)
}

fn destination(
    parent: &Path,
    name: &str,
    conflict: Conflict,
    reserved: &mut HashSet<String>,
) -> Result<PathBuf> {
    valid_name(name)?;
    for index in 0..10_000 {
        let candidate = if index == 0 {
            parent.join(name)
        } else {
            let path = Path::new(name);
            let stem = path.file_stem().unwrap().to_string_lossy();
            let suffix = path
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default();
            parent.join(format!("{stem} ({index}){suffix}"))
        };
        let missing = match fs::symlink_metadata(&candidate) {
            Ok(_) => false,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
            Err(e) => return Err(e.into()),
        };
        if missing && reserved.insert(path_key(&candidate)) {
            return Ok(candidate);
        }
        if conflict == Conflict::Abort {
            bail!(
                "目标重名，未覆盖：{}；可选择自动加序号后重新预览",
                display_path(&candidate)
            );
        }
    }
    bail!("无法生成无冲突的目标名称")
}

pub fn snapshot(path: &Path, cancel: &AtomicBool) -> Result<Vec<Entry>> {
    let path = checked_path(path)?;
    let mut entries = Vec::new();
    let mut stack = vec![(path.clone(), PathBuf::new())];
    while let Some((current, relative)) = stack.pop() {
        cancelled(cancel)?;
        let checked = checked_path(&current)?;
        let meta = fs::symlink_metadata(&checked)?;
        if is_link(&meta) {
            bail!("操作不允许包含链接或重解析点");
        }
        if meta.is_dir() {
            entries.push(Entry {
                relative: relative.clone(),
                directory: true,
                size: 0,
                modified_ms: None,
                hash: None,
            });
            for entry in fs::read_dir(&checked)? {
                cancelled(cancel)?;
                let entry = entry?;
                if entry.file_name() == "session.lock" && entry.file_type()?.is_file() {
                    continue;
                }
                stack.push((entry.path(), relative.join(entry.file_name())));
                if entries.len() + stack.len() > MAX_ENTRIES {
                    bail!("单项超过 100000 个文件/目录，请分开操作");
                }
            }
        } else if meta.is_file() {
            let (size, hash) = hash_file(&checked, cancel)?;
            if meta.len() != size || modified_ms(&checked) != time_ms(&meta) {
                bail!("读取期间文件发生变化：{}", display_path(&checked));
            }
            entries.push(Entry {
                relative,
                directory: false,
                size,
                modified_ms: time_ms(&meta),
                hash: Some(hash),
            });
        } else {
            bail!("不支持非常规文件：{}", display_path(&checked));
        }
        if entries.len() > MAX_ENTRIES {
            bail!("单项超过 100000 个文件/目录，请分开操作");
        }
    }
    entries.sort_by_key(|e| path_key(&e.relative));
    Ok(entries)
}
fn time_ms(meta: &fs::Metadata) -> Option<i64> {
    meta.modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as i64)
}
fn hash_file(path: &Path, flag: &AtomicBool) -> Result<(u64, String)> {
    let mut file = File::open(checked_path(path)?)?;
    let before = file.metadata()?;
    let mut hasher = blake3::Hasher::new();
    let mut bytes = 0;
    let mut buffer = vec![0; 256 * 1024];
    loop {
        cancelled(flag)?;
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        bytes += n as u64;
    }
    let after = file.metadata()?;
    if before.len() != bytes
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
    {
        bail!("文件变化，取消操作：{}", display_path(path));
    }
    Ok((bytes, hasher.finalize().to_hex().to_string()))
}

#[derive(Serialize, Deserialize)]
struct Backup {
    format: u32,
    kind: ItemKind,
    name: String,
    entries: Vec<Entry>,
    source: PathBuf,
}
pub fn prepare_restore(
    policy: &Policy,
    backup: &Path,
    parent: &Path,
    conflict: Conflict,
    cancel: &AtomicBool,
) -> Result<Plan> {
    let backup = checked_path(backup)?;
    let receipt: Backup = serde_json::from_slice(&read_bounded(
        &backup.join("minecraft-explorer-backup.json"),
        RECORD_LIMIT,
    )?)?;
    if receipt.format != 1 || matches!(receipt.kind, ItemKind::Version | ItemKind::Instance) {
        bail!("备份格式不受支持");
    }
    valid_name(&receipt.name)?;
    let source = checked_path(&backup.join("content").join(&receipt.name))?;
    let entries = snapshot(&source, cancel)?;
    if !same_content(&entries, &receipt.entries) {
        bail!("备份校验失败；没有写入目标");
    }
    let parent = policy.target_parent(&receipt.kind, parent, false)?;
    let destination = destination(&parent, &receipt.name, conflict, &mut HashSet::new())?;
    if is_under(&parent, &source) {
        bail!("不能恢复到备份内部");
    }
    let plan = Plan {
        id: unique_id(),
        action: Action::Restore,
        units: vec![Unit {
            item_id: String::new(),
            kind: receipt.kind,
            source,
            destination: Some(destination),
            entries,
            whole_instance: false,
            game_dir: None,
        }],
    };
    plan.check_budget()?;
    Ok(plan)
}
fn same_content(a: &[Entry], b: &[Entry]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(a, b)| {
            a.relative == b.relative
                && a.directory == b.directory
                && a.size == b.size
                && a.hash == b.hash
        })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Outcome {
    pub source: PathBuf,
    pub destination: Option<PathBuf>,
    pub success: bool,
    pub message: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub plan: Plan,
    pub outcomes: Vec<Outcome>,
    pub finished: bool,
}

fn save_record(policy: &Policy, report: &Report) -> Result<()> {
    fs::create_dir_all(&policy.records)?;
    let parent = checked_path(&policy.records)?;
    let target = parent.join(format!("{}.json", report.plan.id));
    let temp = parent.join(format!(".{}.{}.tmp", report.plan.id, unique_id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    serde_json::to_writer_pretty(&mut file, report)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);
    replace_record(&temp, &target)
}
#[cfg(windows)]
fn replace_record(source: &Path, target: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let a: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let b: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe {
        MoveFileExW(
            a.as_ptr(),
            b.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
#[cfg(not(windows))]
fn replace_record(source: &Path, target: &Path) -> Result<()> {
    fs::rename(source, target)?;
    Ok(())
}

/// 即使其他进程在预览后创建目标，也拒绝覆盖。
fn publish(source: &Path, target: &Path) -> Result<()> {
    checked_path(source)?;
    checked_path(target.parent().context("目标无父目录")?)?;
    if fs::symlink_metadata(target).is_ok() {
        bail!("目标在确认后出现，未覆盖：{}", display_path(target));
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};
        let a: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let b: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe { MoveFileExW(a.as_ptr(), b.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    #[cfg(not(windows))]
    {
        bail!("此版本仅在 Windows 验证了不覆盖提交，其他平台未开放写操作");
    }
    Ok(())
}
fn world_locks(units: &[Unit]) -> Result<Vec<File>> {
    let mut paths = HashSet::new();
    let mut locks = Vec::new();
    for unit in units {
        let mut worlds = Vec::new();
        if unit.kind == ItemKind::World {
            worlds.push(unit.source.clone());
        }
        if unit.kind == ItemKind::DataPack
            && let Some(world) = unit.source.parent().and_then(Path::parent)
        {
            worlds.push(world.into());
        }
        if unit.whole_instance {
            let saves = unit.source.join("saves");
            if saves.exists() {
                for entry in fs::read_dir(checked_path(&saves)?)? {
                    let entry = entry?;
                    if entry.file_type()?.is_dir() {
                        worlds.push(entry.path());
                    }
                }
            }
        }
        if unit.kind == ItemKind::DataPack
            && let Some(target) = &unit.destination
            && let Some(world) = target.parent().and_then(Path::parent)
        {
            worlds.push(world.into());
        }
        for world in worlds {
            let lock = world.join("session.lock");
            if !paths.insert(path_key(&lock)) {
                continue;
            }
            match fs::symlink_metadata(&lock) {
                Ok(_) => {
                    let lock = checked_path(&lock)?;
                    let mut options = OpenOptions::new();
                    options.read(true);
                    #[cfg(windows)]
                    {
                        use std::os::windows::fs::OpenOptionsExt;
                        // 共享字节范围锁阻止 Minecraft 获取独占
                        // 会话锁，同时允许 Shell 打开文件更新元信息
                        // 并将其重命名到回收站。
                        options.share_mode(1 | 2 | 4);
                    }
                    let file = options.open(&lock).with_context(|| {
                        format!(
                            "存档可能正在使用或无写权限，请退出游戏：{}",
                            display_path(&world)
                        )
                    })?;
                    file.try_lock_shared().with_context(|| {
                        format!("存档运行锁被占用，请退出游戏：{}", display_path(&world))
                    })?;
                    locks.push(file);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
        }
    }
    Ok(locks)
}

pub fn execute(
    policy: &Policy,
    plan: Plan,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, u64, String),
) -> Result<Report> {
    plan.check_budget()?;
    // 源或目标已经不符合路径策略时，拒绝执行计划。
    for unit in &plan.units {
        if plan.action != Action::Restore {
            policy.guard(&unit.source, unit.kind == ItemKind::Version)?;
            if unit.kind == ItemKind::Version {
                let version = if unit.whole_instance {
                    unit.source.as_path()
                } else {
                    unit.source.parent().context("核心文件缺少版本目录")?
                };
                if !policy.roots.iter().any(|r| {
                    version
                        .parent()
                        .is_some_and(|p| path_key(p) == path_key(&r.join("versions")))
                }) {
                    bail!("版本来源已移除，请重新预览");
                }
                if unit.whole_instance
                    && (unit
                        .game_dir
                        .as_ref()
                        .is_none_or(|p| path_key(p) != path_key(version))
                        || !policy
                            .games
                            .iter()
                            .any(|p| path_key(p) == path_key(version)))
                {
                    bail!("独立实例关联已变化，请重新预览");
                }
            } else {
                let mut item = crate::discovery::base_item(
                    unit.kind.clone(),
                    unit.source.clone(),
                    PathBuf::new(),
                    String::new(),
                );
                item.game_dir = unit.game_dir.clone();
                policy.leaf(&item)?;
            }
        } else {
            checked_path(&unit.source)?;
        }
        if let Some(target) = &unit.destination {
            let parent = checked_path(target.parent().context("目标没有父目录")?)?;
            if plan.action != Action::Rename {
                policy.target_parent(&unit.kind, &parent, plan.action == Action::Backup)?;
            }
            if is_under(&parent, &unit.source) {
                bail!("目标不能在源路径内部");
            }
        }
    }
    drop(world_locks(&plan.units)?);
    let mut report = Report {
        plan,
        outcomes: Vec::new(),
        finished: false,
    };
    save_record(policy, &report)?;
    let total = report
        .plan
        .units
        .iter()
        .flat_map(|u| &u.entries)
        .map(|e| e.size)
        .sum::<u64>()
        .max(1);
    let mut done = 0;
    for (unit_index, unit) in report.plan.units.clone().into_iter().enumerate() {
        let result = (|| -> Result<()> {
            cancelled(cancel)?;
            let mut locks = world_locks(std::slice::from_ref(&unit))?;
            if snapshot(&unit.source, cancel)? != unit.entries {
                bail!(
                    "确认后源文件发生变化，请重新预览：{}",
                    display_path(&unit.source)
                );
            }
            if report.plan.action == Action::Recycle {
                // Shell 需要打开 session.lock 记录信息；真实
                // 游戏的独占锁仍由 Windows 强制执行。
                locks.clear();
                recycle::send(&unit.source, cancel)?;
            } else if report.plan.action == Action::Rename {
                let target = unit.destination.as_ref().context("缺少重命名目标")?;
                locks.clear();
                publish(&unit.source, target)?;
            } else {
                let target = unit.destination.as_ref().context("缺少目标")?;
                let parent = checked_path(target.parent().context("目标缺少父目录")?)?;
                let stage = parent.join(format!(
                    ".minecraft-explorer-{}-{}.partial",
                    report.plan.id, unit_index
                ));
                fs::create_dir(&stage)?;
                let payload = stage.join("payload");
                let result = (|| -> Result<()> {
                    copy_tree(&unit.source, &payload, &unit.entries, cancel, |bytes| {
                        done += bytes;
                        progress(
                            done,
                            total,
                            format!(
                                "{} · {}",
                                report.plan.action.label(),
                                display_path(&unit.source)
                            ),
                        );
                    })?;
                    if !same_content(&snapshot(&payload, cancel)?, &unit.entries) {
                        bail!("目标副本校验失败，原件保留");
                    }
                    if snapshot(&unit.source, cancel)? != unit.entries {
                        bail!("源在复制期间发生变化，原件保留");
                    }
                    cancelled(cancel)?;
                    if report.plan.action == Action::Backup {
                        let name = unit
                            .source
                            .file_name()
                            .context("源缺少名称")?
                            .to_string_lossy()
                            .into_owned();
                        let content = stage.join("content");
                        fs::create_dir(&content)?;
                        publish(&payload, &content.join(&name))?;
                        let receipt = Backup {
                            format: 1,
                            kind: unit.kind.clone(),
                            name,
                            entries: unit.entries.clone(),
                            source: unit.source.clone(),
                        };
                        let mut file = OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(stage.join("minecraft-explorer-backup.json"))?;
                        serde_json::to_writer_pretty(&mut file, &receipt)?;
                        file.sync_all()?;
                        drop(file);
                        publish(&stage, target)?;
                    } else {
                        publish(&payload, target)?;
                    }
                    if report.plan.action == Action::Move {
                        // 处理源文件前，先持久化已经校验的目标。
                        report.outcomes.push(Outcome {
                            source: unit.source.clone(),
                            destination: Some(target.clone()),
                            success: false,
                            message: "目标副本已校验提交；正在将原件移入回收站".into(),
                        });
                        save_record(policy, &report)?;
                        cancelled(cancel)?;
                        if snapshot(&unit.source, cancel)? != unit.entries {
                            bail!("原件变化；保留源与已提交目标，请检查两份数据");
                        }
                        locks.clear();
                        recycle::send(&unit.source, cancel).context(
                            "目标已保留；原件无法移入回收站，未永久删除，请检查两份数据",
                        )?;
                        report.outcomes.pop();
                    }
                    Ok(())
                })();
                if stage.exists()
                    && let Err(e) = cleanup_stage(&stage)
                {
                    return Err(e).context(format!(
                        "临时副本保留在 {}；原件和已提交目标请按操作记录检查",
                        display_path(&stage)
                    ));
                }
                result?;
            }
            Ok(())
        })();
        let success = result.is_ok();
        report.outcomes.push(Outcome {
            source: unit.source.clone(),
            destination: unit.destination.clone(),
            success,
            message: match result {
                Ok(()) => format!("{}完成", report.plan.action.label()),
                Err(e) => format!("{e:#}"),
            },
        });
        save_record(policy, &report)?;
        if !success || cancel.load(Ordering::SeqCst) {
            break;
        }
    }
    report.finished = report.outcomes.len() == report.plan.units.len()
        && report.outcomes.iter().all(|o| o.success);
    save_record(policy, &report)?;
    Ok(report)
}

fn copy_tree(
    source: &Path,
    target: &Path,
    entries: &[Entry],
    flag: &AtomicBool,
    mut advance: impl FnMut(u64),
) -> Result<()> {
    let directory = entries
        .first()
        .is_some_and(|e| e.relative.as_os_str().is_empty() && e.directory);
    if directory {
        fs::create_dir(target)?;
    }
    for entry in entries {
        cancelled(flag)?;
        let input = if entry.relative.as_os_str().is_empty() {
            source.to_path_buf()
        } else {
            source.join(&entry.relative)
        };
        let output = if entry.relative.as_os_str().is_empty() {
            target.to_path_buf()
        } else {
            target.join(&entry.relative)
        };
        if entry.directory {
            if !entry.relative.as_os_str().is_empty() {
                fs::create_dir(&output)?;
            }
            continue;
        }
        let mut reader = File::open(checked_path(&input)?)?;
        let before = reader.metadata()?;
        let mut writer = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)?;
        let mut hash = blake3::Hasher::new();
        let mut buffer = vec![0; 256 * 1024];
        let mut size = 0;
        loop {
            cancelled(flag)?;
            let n = reader.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            writer.write_all(&buffer[..n])?;
            hash.update(&buffer[..n]);
            size += n as u64;
            advance(n as u64);
        }
        writer.sync_all()?;
        if size != entry.size
            || Some(hash.finalize().to_hex().to_string()) != entry.hash
            || before.modified().ok() != reader.metadata()?.modified().ok()
        {
            bail!("复制期间源文件变化，未提交目标");
        }
        if let Ok(modified) = before.modified() {
            writer.set_times(fs::FileTimes::new().set_modified(modified))?;
        }
    }
    Ok(())
}
fn cleanup_stage(path: &Path) -> Result<()> {
    let checked = checked_path(path)?;
    if !checked.file_name().is_some_and(|n| {
        n.to_string_lossy().starts_with(".minecraft-explorer-")
            && n.to_string_lossy().ends_with(".partial")
    }) {
        bail!("拒绝清理非本程序临时目录");
    }
    // 遇到重解析点则拒绝，清理时不递归跟随。
    let mut stack = vec![checked.clone()];
    while let Some(dir) = stack.pop() {
        checked_path(&dir)?;
        for child in fs::read_dir(&dir)? {
            let child = child?;
            let meta = fs::symlink_metadata(child.path())?;
            if is_link(&meta) {
                bail!("临时目录出现链接，保留供检查");
            }
            if meta.is_dir() {
                stack.push(child.path());
            }
        }
    }
    fs::remove_dir_all(checked)?;
    Ok(())
}
pub fn history(policy: &Policy) -> Result<String> {
    if !policy.records.exists() {
        return Ok(
            "尚无文件操作记录。备份可在此页恢复；回收站内容请通过 Windows 回收站还原。".into(),
        );
    }
    let mut paths = fs::read_dir(checked_path(&policy.records)?)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect::<Vec<_>>();
    paths.sort();
    paths.reverse();
    let mut text = String::new();
    for path in paths.into_iter().take(30) {
        match read_bounded(&path, RECORD_LIMIT)
            .and_then(|bytes| Ok(serde_json::from_slice::<Report>(&bytes)?))
        {
            Ok(report) => {
                text.push_str(&format!(
                    "\n{} · {} · {}\n",
                    report.plan.id,
                    report.plan.action.label(),
                    if report.finished {
                        "已完成"
                    } else {
                        "未全部完成，请检查源、目标及临时副本"
                    }
                ));
                for (n, unit) in report.plan.units.iter().enumerate() {
                    text.push_str(&format!(
                        "计划源：{}\n计划目标：{}\n",
                        display_path(&unit.source),
                        unit.destination
                            .as_ref()
                            .map(|p| display_path(p))
                            .unwrap_or_else(|| "Windows 回收站".into())
                    ));
                    if let Some(target) = &unit.destination
                        && let Some(parent) = target.parent()
                    {
                        let stage = parent.join(format!(
                            ".minecraft-explorer-{}-{}.partial",
                            report.plan.id, n
                        ));
                        if stage.exists() {
                            text.push_str(&format!(
                                "未清理临时副本：{}（请先核对原件和目标，再处理此目录）\n",
                                display_path(&stage)
                            ));
                        }
                    }
                }
                for outcome in report.outcomes {
                    text.push_str(&format!(
                        "{} → {}\n{}\n",
                        display_path(&outcome.source),
                        outcome
                            .destination
                            .as_ref()
                            .map(|p| display_path(p))
                            .unwrap_or_else(|| "回收站".into()),
                        outcome.message
                    ));
                }
            }
            Err(e) => text.push_str(&format!("\n记录不可读取 {}：{e}\n", display_path(&path))),
        }
    }
    Ok(text)
}
