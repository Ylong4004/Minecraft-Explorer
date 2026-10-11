use crate::{content, discovery::*, minecraft::world_metadata, models::*, platform::*};
use anyhow::{Result, bail};
use std::{
    fs,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Debug)]
pub enum ScanEvent {
    Discovered(Vec<Item>),
    Updated(Box<Item>),
    Progress {
        done: usize,
        total: usize,
        message: String,
    },
    Warning(String),
}

#[derive(Default, Debug)]
pub struct ScanReport {
    pub completed_sources: Vec<String>,
    pub failed_sources: Vec<String>,
    pub offline_sources: Vec<String>,
    pub warnings: Vec<String>,
    pub items: Vec<Item>,
    pub cancelled: bool,
}

fn cancelled(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        bail!("扫描已取消");
    }
    Ok(())
}

/// Iterative traversal avoids stack overflow on deep directories, never follows links.
pub fn directory_size(
    path: &Path,
    kind: &ItemKind,
    is_root: bool,
    cancel: &AtomicBool,
) -> Result<(u64, bool)> {
    checked_path(path)?;
    cancelled(cancel)?;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_file() {
        return Ok((metadata.len(), true));
    }
    let mut stack = vec![path.to_path_buf()];
    let mut bytes = 0u64;
    let mut complete = true;
    while let Some(dir) = stack.pop() {
        cancelled(cancel)?;
        // Check again immediately before opening: links may have changed since discovery.
        if checked_path(&dir).is_err() {
            complete = false;
            continue;
        }
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => {
                complete = false;
                continue;
            }
        };
        for entry in entries {
            cancelled(cancel)?;
            let Ok(entry) = entry else {
                complete = false;
                continue;
            };
            let child = entry.path();
            let Ok(meta) = fs::symlink_metadata(&child) else {
                complete = false;
                continue;
            };
            if is_link(&meta) {
                complete = false;
                continue;
            }
            let top = dir == path;
            let core_file = top
                && meta.is_file()
                && child.extension().is_some_and(|v| v == "json" || v == "jar");
            match kind {
                ItemKind::Version if !core_file => continue,
                ItemKind::Instance
                    if (is_root && top && entry.file_name() == "versions")
                        || (!is_root && core_file) =>
                {
                    continue;
                }
                _ => (),
            }
            if meta.is_dir() {
                stack.push(child);
            } else if meta.is_file() {
                bytes = bytes.saturating_add(meta.len());
            }
        }
    }
    Ok((bytes, complete))
}

pub fn scan(
    settings: &Settings,
    cancel: &AtomicBool,
    mut emit: impl FnMut(ScanEvent),
) -> ScanReport {
    scan_items(discover(settings, cancel), cancel, &mut emit)
}

fn scan_items(
    mut discovered: Discovery,
    cancel: &AtomicBool,
    mut emit: impl FnMut(ScanEvent),
) -> ScanReport {
    // World metadata is the first useful result; do not wait for a large root size walk.
    discovered.items.sort_by_key(|i| match i.kind {
        ItemKind::World => 0,
        ItemKind::Version => 1,
        ItemKind::Instance => 2,
        _ => 1,
    });
    let mut report = ScanReport {
        completed_sources: discovered.completed_sources,
        failed_sources: discovered.failed_sources,
        offline_sources: discovered.offline_sources,
        warnings: discovered.warnings,
        ..Default::default()
    };
    for warning in &report.warnings {
        emit(ScanEvent::Warning(warning.clone()));
    }
    emit(ScanEvent::Discovered(discovered.items.clone()));
    let total = discovered.items.len();
    for (index, mut item) in discovered.items.into_iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            report.cancelled = true;
            break;
        }
        emit(ScanEvent::Progress {
            done: index,
            total,
            message: format!("读取元信息与大小 · {}", item.name),
        });
        if item.kind == ItemKind::World {
            let meta = world_metadata(&item.path);
            if let Some(name) = meta.name {
                item.name = name;
            }
            item.last_played_ms = meta.last_played_ms;
            item.game_version = meta.game_version;
            item.metadata_status = meta.status;
            let icon = item.path.join("icon.png");
            if read_bounded(&icon, 2 * 1024 * 1024).is_ok() {
                item.preview_modified_ms = modified_ms(&icon);
                item.icon = Some(icon);
            }
        } else if matches!(
            item.kind,
            ItemKind::ResourcePack | ItemKind::Mod | ItemKind::ShaderPack | ItemKind::DataPack
        ) {
            let meta = content::metadata(&item.path, &item.kind);
            if let Some(name) = meta.display_name {
                item.name = name;
            }
            item.metadata_status = meta.status;
            item.content = Some(meta.metadata);
            if item.path.is_dir()
                && let Some(entry) = item.content.as_ref().and_then(|m| m.preview_entry.as_ref())
            {
                let icon = item.path.join(entry);
                if read_bounded(&icon, 2 * 1024 * 1024).is_ok() {
                    item.preview_modified_ms = modified_ms(&icon);
                    item.icon = Some(icon);
                }
            }
        }
        match directory_size(&item.path, &item.kind, item.path == item.root, cancel) {
            Ok((bytes, complete)) => {
                item.size = Some(bytes);
                item.size_complete = complete;
                if !complete {
                    item.metadata_status
                        .push_str("；大小为下限：部分文件不可访问或跳过链接");
                }
            }
            Err(_) if cancel.load(Ordering::Relaxed) => {
                report.cancelled = true;
                break;
            }
            Err(e) => {
                item.available = false;
                item.metadata_status
                    .push_str(&format!("；大小计算失败：{e}"));
            }
        }
        emit(ScanEvent::Updated(Box::new(item.clone())));
        report.items.push(item);
    }
    report.cancelled |= cancel.load(Ordering::Relaxed);
    report
}

pub struct RefreshReport {
    pub scan: ScanReport,
    pub removed_ids: Vec<String>,
    pub invalidated: Vec<Item>,
}

/// Rediscover membership, but only read metadata/size for changed leaf items.
/// Aggregate instance sizes become pending until an explicit full refresh.
pub fn refresh_changed(
    settings: &Settings,
    cached: &[Item],
    paths: &[std::path::PathBuf],
    cancel: &AtomicBool,
    emit: impl FnMut(ScanEvent),
) -> RefreshReport {
    use std::collections::HashSet;
    let mut discovery = discover(settings, cancel);
    let ids: HashSet<_> = discovery.items.iter().map(|i| i.id.clone()).collect();
    let touched = |item: &Item| {
        paths.iter().any(|p| {
            let path = path_key(p);
            let own = path_key(&item.path);
            path == own
                || path.starts_with(&(own.clone() + "\\"))
                || own.starts_with(&(path + "\\"))
        })
    };
    let mut invalidated = Vec::new();
    discovery.items.retain(|item| {
        let existing = cached.iter().find(|c| c.id == item.id);
        let affected = touched(item) || existing.is_none();
        if !affected {
            return false;
        }
        if item.kind == ItemKind::Instance {
            let mut instance = existing.cloned().unwrap_or_else(|| item.clone());
            instance.size = None;
            instance.size_complete = false;
            instance.metadata_status =
                "目录内容已变化；实例总大小待完整刷新，叶条目已增量更新".into();
            invalidated.push(instance);
            return false;
        }
        true
    });
    let removed_ids = cached
        .iter()
        .filter(|i| {
            !ids.contains(&i.id)
                && touched(i)
                && discovery.completed_sources.contains(&i.source)
                && std::fs::symlink_metadata(&i.path)
                    .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
        })
        .map(|i| i.id.clone())
        .collect();
    RefreshReport {
        scan: scan_items(discovery, cancel, emit),
        removed_ids,
        invalidated,
    }
}
