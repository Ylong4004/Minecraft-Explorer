use crate::{minecraft::version_metadata, models::*, platform::*};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Default)]
pub struct Discovery {
    pub items: Vec<Item>,
    pub completed_sources: Vec<String>,
    pub failed_sources: Vec<String>,
    pub offline_sources: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn folder_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

pub fn base_item(kind: ItemKind, path: PathBuf, root: PathBuf, source: String) -> Item {
    Item {
        id: format!("{}:{}", kind.as_str(), path_key(&path)),
        name: folder_name(&path),
        folder: folder_name(&path),
        modified_ms: modified_ms(&path),
        path,
        root,
        source,
        kind,
        game_dir: None,
        ownership: "未识别实例".into(),
        version_id: None,
        game_version: None,
        loader: None,
        size: None,
        size_complete: false,
        last_played_ms: None,
        metadata_status: String::new(),
        icon: None,
        preview_modified_ms: None,
        available: true,
        content: None,
        favorite: false,
        tags: Vec::new(),
        last_opened_ms: None,
    }
}

pub fn children(path: &Path, warnings: &mut Vec<String>) -> Vec<PathBuf> {
    match fs::read_dir(path) {
        Ok(entries) => {
            let mut result = Vec::new();
            for entry in entries {
                match entry {
                    Ok(entry) => {
                        let path = entry.path();
                        match entry.metadata() {
                            Ok(meta) if !is_link(&meta) => result.push(path),
                            Ok(_) => warnings
                                .push(format!("跳过链接 / 重解析点：{}", display_path(&path))),
                            Err(e) => warnings.push(format!("{}：{e}", display_path(&path))),
                        }
                    }
                    Err(e) => warnings.push(format!("{}：{e}", display_path(path))),
                }
            }
            result.sort();
            result
        }
        Err(e) => {
            warnings.push(format!("目录不可访问 {}：{e}", display_path(path)));
            Vec::new()
        }
    }
}

pub fn discover(settings: &Settings, cancel: &AtomicBool) -> Discovery {
    let mut out = Discovery::default();
    let mut seen_games = HashSet::new();
    let mut seen_versions = HashSet::new();
    // 先处理根目录，自定义别名不能改变已确认的归属。
    let mut sources = settings.sources.clone();
    sources.sort_by_key(|s| if s.kind == SourceKind::Root { 0 } else { 1 });
    for source in &sources {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let source_key = path_key(&source.path);
        let path = match checked_path(&source.path) {
            Ok(path) if path.is_dir() => path,
            Ok(_) => {
                out.offline_sources.push(source_key.clone());
                out.failed_sources.push(source_key);
                out.warnings
                    .push(format!("不是目录：{}", display_path(&source.path)));
                continue;
            }
            Err(e) => {
                out.offline_sources.push(source_key.clone());
                out.failed_sources.push(source_key);
                out.warnings.push(e.to_string());
                continue;
            }
        };
        if fs::read_dir(&path).is_err() {
            out.offline_sources.push(source_key.clone());
            out.failed_sources.push(source_key);
            out.warnings
                .push(format!("无权读取目录：{}", display_path(&path)));
            continue;
        }
        out.completed_sources.push(source_key.clone());
        let owner_root = source
            .associated_root
            .as_ref()
            .map(|p| checked_path(p).unwrap_or_else(|_| p.clone()))
            .unwrap_or_else(|| path.clone());
        let mut games = vec![(
            path.clone(),
            if source.kind == SourceKind::Root {
                "共享目录 / 根目录文件".to_string()
            } else {
                "自定义游戏目录（手动添加）".to_string()
            },
        )];
        if source.kind == SourceKind::Root {
            let versions = path.join("versions");
            if versions.exists() {
                if let Err(e) = checked_path(&versions) {
                    out.warnings.push(e.to_string());
                    out.failed_sources.push(source_key.clone());
                } else if fs::read_dir(&versions).is_err() {
                    out.warnings
                        .push(format!("版本目录不可访问：{}", display_path(&versions)));
                    out.failed_sources.push(source_key.clone());
                } else {
                    for dir in children(&versions, &mut out.warnings) {
                        if !dir.is_dir() {
                            continue;
                        }
                        if cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        if !seen_versions.insert(path_key(&dir)) {
                            continue;
                        }
                        if fs::read_dir(&dir).is_err() {
                            out.failed_sources.push(source_key.clone());
                        }
                        let files = children(&dir, &mut out.warnings);
                        let expected = dir.join(format!("{}.json", folder_name(&dir)));
                        let jsons: Vec<_> = files
                            .iter()
                            .filter(|p| p.extension().is_some_and(|e| e == "json"))
                            .cloned()
                            .collect();
                        let definition = if jsons.contains(&expected) {
                            Some(expected)
                        } else if jsons.len() == 1 {
                            jsons.first().cloned()
                        } else {
                            None
                        };
                        let mut version = base_item(
                            ItemKind::Version,
                            dir.clone(),
                            path.clone(),
                            source_key.clone(),
                        );
                        if let Some(json) = definition {
                            let meta = version_metadata(&json);
                            version.version_id = meta.id;
                            version.game_version = meta.inherits;
                            version.loader = meta.loader;
                            version.metadata_status = meta.status;
                            if let Some(id) = &version.version_id {
                                version.name = id.clone();
                            }
                        } else {
                            version.metadata_status =
                                "未识别实例：无唯一版本 JSON；不猜测版本 ID".into();
                        }
                        let independent = [
                            "saves",
                            "mods",
                            "resourcepacks",
                            "shaderpacks",
                            "screenshots",
                        ]
                        .iter()
                        .any(|name| {
                            files
                                .iter()
                                .any(|p| p.file_name().is_some_and(|v| v == *name) && p.is_dir())
                        });
                        if independent {
                            version.game_dir = Some(dir.clone());
                            version.ownership = "独立内容目录".into();
                            games.push((dir.clone(), format!("独立目录 · {}", version.name)));
                        } else {
                            version.ownership = "游戏目录未确认；可手动关联".into();
                        }
                        out.items.push(version);
                    }
                }
            }
            // 仅提取非认证字段，不反序列化账号或令牌。
            let profile = path.join("launcher_profiles.json");
            if profile.exists() {
                match read_bounded(&profile, 4 * 1024 * 1024)
                    .and_then(|b| Ok(serde_json::from_slice::<Profiles>(&b)?))
                {
                    Ok(profiles) => {
                        for (_, profile) in profiles.profiles {
                            if let Some(game) = profile.game_dir {
                                let game = PathBuf::from(game);
                                match checked_path(&game) {
                                    Ok(game) if game.is_dir() => games.push((
                                        game,
                                        format!(
                                            "官方自定义 gameDir · {}",
                                            profile.name.unwrap_or_default()
                                        ),
                                    )),
                                    Ok(_) => out
                                        .warnings
                                        .push(format!("gameDir 不是目录：{}", game.display())),
                                    Err(e) => out.warnings.push(e.to_string()),
                                }
                            }
                        }
                    }
                    Err(e) => out.warnings.push(format!("官方配置读取失败：{e}")),
                }
            }
        }
        for (game, ownership) in games {
            if !seen_games.insert(path_key(&game)) {
                continue;
            }
            let mut instance = base_item(
                ItemKind::Instance,
                game.clone(),
                owner_root.clone(),
                source_key.clone(),
            );
            instance.game_dir = Some(game.clone());
            instance.ownership = ownership.clone();
            instance.metadata_status =
                "实际游戏目录；大小为逻辑字节，根目录不包含 versions，独立目录不包含顶层 JSON/JAR"
                    .into();
            out.items.push(instance);
            for (folder, kind) in [
                ("resourcepacks", ItemKind::ResourcePack),
                ("mods", ItemKind::Mod),
                ("shaderpacks", ItemKind::ShaderPack),
            ] {
                discover_content(
                    &game.join(folder),
                    kind,
                    &game,
                    &owner_root,
                    &source_key,
                    &ownership,
                    &mut out,
                    cancel,
                );
            }
            let saves = game.join("saves");
            if !saves.exists() {
                continue;
            }
            if let Err(e) = checked_path(&saves) {
                out.warnings.push(e.to_string());
                out.failed_sources.push(source_key.clone());
                continue;
            }
            let before = out.warnings.len();
            let worlds = children(&saves, &mut out.warnings);
            // 无法访问 saves 时，不清除之前索引的世界。
            if out.warnings.len() > before && fs::read_dir(&saves).is_err() {
                out.failed_sources.push(source_key.clone());
            }
            for world in worlds {
                if !world.is_dir() || folder_name(&world).starts_with(".minecraft-explorer-") {
                    continue;
                }
                let mut item = base_item(
                    ItemKind::World,
                    world,
                    owner_root.clone(),
                    source_key.clone(),
                );
                item.game_dir = Some(game.clone());
                item.ownership = ownership.clone();
                if let Some(version) = out
                    .items
                    .iter()
                    .find(|v| v.kind == ItemKind::Version && v.game_dir.as_ref() == Some(&game))
                {
                    item.version_id = version.version_id.clone();
                    item.loader = version.loader.clone();
                }
                let world_path = item.path.clone();
                out.items.push(item);
                discover_content(
                    &world_path.join("datapacks"),
                    ItemKind::DataPack,
                    &game,
                    &owner_root,
                    &source_key,
                    &format!("{ownership} · 世界 {}", folder_name(&world_path)),
                    &mut out,
                    cancel,
                );
            }
        }
    }
    out.completed_sources
        .retain(|s| !out.failed_sources.contains(s));
    out
}

#[allow(clippy::too_many_arguments)]
fn discover_content(
    folder: &Path,
    kind: ItemKind,
    game: &Path,
    root: &Path,
    source: &str,
    ownership: &str,
    out: &mut Discovery,
    cancel: &AtomicBool,
) {
    if !folder.exists() {
        return;
    }
    if let Err(e) = checked_path(folder) {
        out.warnings.push(e.to_string());
        out.failed_sources.push(source.into());
        return;
    }
    if fs::read_dir(folder).is_err() {
        out.warnings
            .push(format!("内容目录不可访问：{}", display_path(folder)));
        out.failed_sources.push(source.into());
        return;
    }
    for path in children(folder, &mut out.warnings) {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if folder_name(&path).starts_with(".minecraft-explorer-") {
            continue;
        }
        let extension = path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        let accepted = path.is_dir()
            || if kind == ItemKind::Mod {
                extension == "jar" || folder_name(&path).to_lowercase().ends_with(".jar.disabled")
            } else {
                extension == "zip"
            };
        if !accepted {
            continue;
        }
        if kind == ItemKind::DataPack {
            match crate::content::is_data_pack(&path) {
                Ok(true) => (),
                Ok(false) => continue,
                Err(e) => {
                    out.warnings
                        .push(format!("数据包结构无法验证 {}：{e}", display_path(&path)));
                    // 候选暂时不可读时，保留旧索引。
                    if !path.is_file()
                        || e.downcast_ref::<std::io::Error>()
                            .is_some_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied)
                    {
                        out.failed_sources.push(source.into());
                    }
                    continue;
                }
            }
        }
        let mut item = base_item(kind.clone(), path, root.to_path_buf(), source.into());
        item.game_dir = Some(game.into());
        item.ownership = ownership.into();
        if let Some(version) = out
            .items
            .iter()
            .find(|v| v.kind == ItemKind::Version && v.game_dir.as_deref() == Some(game))
        {
            item.version_id = version.version_id.clone();
            item.loader = version.loader.clone();
        }
        out.items.push(item);
    }
}

#[derive(serde::Deserialize)]
struct Profiles {
    #[serde(default)]
    profiles: std::collections::HashMap<String, Profile>,
}
#[derive(serde::Deserialize)]
struct Profile {
    #[serde(rename = "gameDir")]
    game_dir: Option<String>,
    name: Option<String>,
}
