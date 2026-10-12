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
    Metadata(Box<Item>),
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

/// 一次扫描内缓存已完成的目录子树，实例、世界和目录包共用统计。
#[derive(Default)]
pub struct SizeCache {
    directories: std::collections::HashMap<std::path::PathBuf, (u64, bool)>,
    pub enumerated_directories: usize,
}

impl SizeCache {
    pub fn size(
        &mut self,
        path: &Path,
        kind: &ItemKind,
        is_root: bool,
        cancel: &AtomicBool,
        mut progress: impl FnMut(u64),
    ) -> Result<(u64, bool)> {
        use crate::walk::{self, Event};
        checked_path(path)?;
        cancelled(cancel)?;
        let metadata = fs::symlink_metadata(path)?;
        if metadata.is_file() {
            return Ok((metadata.len(), true));
        }
        let filtered = matches!(kind, ItemKind::Instance | ItemKind::Version);
        if !filtered && let Some(value) = self.directories.get(path) {
            return Ok(*value);
        }
        struct Total {
            path: std::path::PathBuf,
            parent: Option<usize>,
            bytes: u64,
            complete: bool,
            cache: bool,
        }
        let mut totals = Vec::<Total>::new();
        let mut files = 0u64;
        let interrupted = walk::walk(path, cancel, |event| {
            match event {
                Event::Directory {
                    id,
                    parent,
                    path: child,
                } => {
                    let excluded = parent == Some(0)
                        && (kind == &ItemKind::Version
                            || (kind == &ItemKind::Instance
                                && is_root
                                && child.file_name().is_some_and(|n| n == "versions")));
                    let cached = if excluded || (id == 0 && filtered) {
                        None
                    } else {
                        self.directories.get(child).copied()
                    };
                    totals.push(Total {
                        path: child.into(),
                        parent,
                        bytes: cached.map_or(0, |v| v.0),
                        complete: cached.is_none_or(|v| v.1),
                        cache: !excluded && !(id == 0 && filtered),
                    });
                    if excluded || cached.is_some() {
                        return false;
                    }
                    self.enumerated_directories += 1;
                }
                Event::File {
                    parent,
                    path: child,
                    bytes,
                } => {
                    files += 1;
                    let core =
                        parent == 0 && child.extension().is_some_and(|v| v == "json" || v == "jar");
                    if !(parent == 0
                        && ((kind == &ItemKind::Version && !core)
                            || (kind == &ItemKind::Instance && !is_root && core)))
                    {
                        totals[parent].bytes = totals[parent].bytes.saturating_add(bytes);
                    }
                    if files.is_multiple_of(1024) {
                        progress(files);
                    }
                }
                Event::Skipped { parent } => totals[parent].complete = false,
                Event::End { id } => {
                    let total = &totals[id];
                    if total.cache {
                        self.directories
                            .insert(total.path.clone(), (total.bytes, total.complete));
                    }
                    if let Some(parent) = total.parent {
                        let bytes = total.bytes;
                        let complete = total.complete;
                        totals[parent].bytes = totals[parent].bytes.saturating_add(bytes);
                        totals[parent].complete &= complete;
                    }
                }
            }
            true
        })?;
        if interrupted {
            bail!("扫描已取消");
        }
        Ok((totals[0].bytes, totals[0].complete))
    }
}

/// 单独统计时也采用同一只读枚举器；不跟随链接，不读取文件内容。
pub fn directory_size(
    path: &Path,
    kind: &ItemKind,
    is_root: bool,
    cancel: &AtomicBool,
) -> Result<(u64, bool)> {
    SizeCache::default().size(path, kind, is_root, cancel, |_| {})
}

pub fn scan(
    settings: &Settings,
    cancel: &AtomicBool,
    mut emit: impl FnMut(ScanEvent),
) -> ScanReport {
    scan_items(discover(settings, cancel), cancel, &mut emit)
}

/// 发现结束后立即清除已证实错误的数据包分类，
/// 无需等待大世界的大小统计；不可读与
/// 离线路径保留缓存，用户注释独立保存。
pub fn invalid_data_pack_cache(cached: &[Item], discovered: &[Item]) -> Vec<String> {
    let valid = discovered
        .iter()
        .filter(|i| i.kind == ItemKind::DataPack)
        .map(|i| &i.id)
        .collect::<std::collections::HashSet<_>>();
    cached
        .iter()
        .filter(|i| {
            i.kind == ItemKind::DataPack
                && !valid.contains(&i.id)
                && content::is_data_pack(&i.path).is_ok_and(|valid| !valid)
        })
        .map(|i| i.id.clone())
        .collect()
}

fn scan_items(
    mut discovered: Discovery,
    cancel: &AtomicBool,
    mut emit: impl FnMut(ScanEvent),
) -> ScanReport {
    // 优先提供世界元信息，不等待大型根目录的大小统计。
    discovered.items.sort_by_key(|i| match i.kind {
        ItemKind::World => 1,
        ItemKind::Version => 0,
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
    let mut ready = Vec::new();
    for (index, mut item) in discovered.items.into_iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            report.cancelled = true;
            break;
        }
        emit(ScanEvent::Progress {
            done: index,
            total,
            message: format!("读取元信息 · {}", item.name),
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
        }
        if !matches!(
            item.kind,
            ItemKind::ResourcePack | ItemKind::Mod | ItemKind::ShaderPack | ItemKind::DataPack
        ) {
            emit(ScanEvent::Metadata(Box::new(item.clone())));
        }
        ready.push(item);
    }
    ready.sort_by_key(|i| match i.kind {
        ItemKind::Version => 0,
        ItemKind::Instance if i.path != i.root => 1,
        ItemKind::Instance => 2,
        _ => 3,
    });
    let mut sizes = SizeCache::default();
    let boundary = ready
        .iter()
        .position(|i| !matches!(i.kind, ItemKind::Version | ItemKind::Instance))
        .unwrap_or(ready.len());
    let remaining = ready.split_off(boundary);
    for item in ready {
        update_size(item, total, &mut sizes, cancel, &mut report, &mut emit);
    }
    let packages = remaining.iter().filter(|i| is_content(&i.kind)).count();
    if packages >= 16 && !cancel.load(Ordering::Relaxed) {
        use std::{
            collections::VecDeque,
            sync::{Mutex, mpsc},
        };
        let queue = Mutex::new(VecDeque::from(remaining));
        let (sender, receiver) = mpsc::sync_channel(16);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let sender = sender.clone();
                let queue = &queue;
                scope.spawn(move || {
                    while !cancel.load(Ordering::Relaxed) {
                        let Some(mut item) = queue.lock().unwrap().pop_front() else {
                            break;
                        };
                        if is_content(&item.kind) {
                            read_content_metadata(&mut item);
                        }
                        if sender.send(item).is_err() {
                            break;
                        }
                    }
                });
            }
            drop(sender);
            // 取消后继续接收在途结果但不提交，确保有界队列的发送线程正常退出。
            for item in receiver {
                update_size(item, total, &mut sizes, cancel, &mut report, &mut emit);
            }
        });
    } else {
        for mut item in remaining {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            if is_content(&item.kind) {
                read_content_metadata(&mut item);
            }
            update_size(item, total, &mut sizes, cancel, &mut report, &mut emit);
        }
    }
    report.cancelled |= cancel.load(Ordering::Relaxed);
    report
}

fn is_content(kind: &ItemKind) -> bool {
    matches!(
        kind,
        ItemKind::ResourcePack | ItemKind::Mod | ItemKind::ShaderPack | ItemKind::DataPack
    )
}

fn update_size(
    mut item: Item,
    total: usize,
    sizes: &mut SizeCache,
    cancel: &AtomicBool,
    report: &mut ScanReport,
    mut emit: impl FnMut(ScanEvent),
) {
    if cancel.load(Ordering::Relaxed) {
        report.cancelled = true;
        return;
    }
    let index = report.items.len();
    let message = format!("统计目录大小 · {}", item.name);
    emit(ScanEvent::Progress {
        done: index,
        total,
        message: message.clone(),
    });
    match sizes.size(
        &item.path,
        &item.kind,
        item.path == item.root,
        cancel,
        |files| {
            emit(ScanEvent::Progress {
                done: index,
                total,
                message: format!("{message} · 已读取 {files} 个文件"),
            });
        },
    ) {
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
            return;
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

fn read_content_metadata(item: &mut Item) {
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

pub struct RefreshReport {
    pub scan: ScanReport,
    pub removed_ids: Vec<String>,
    pub invalidated: Vec<Item>,
}

/// 重新发现成员关系，只读取变化叶条目的元信息和大小。
/// 受影响的实例在同一个后台任务内补齐汇总，其他实例不重新统计。
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
    let invalidated = Vec::new();
    discovery.items.retain(|item| {
        let existing = cached.iter().find(|c| c.id == item.id);
        let affected = touched(item) || existing.is_none();
        if !affected {
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
                && (std::fs::symlink_metadata(&i.path)
                    .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
                    || (i.kind == ItemKind::DataPack
                        && crate::content::is_data_pack(&i.path).is_ok_and(|valid| !valid)))
        })
        .map(|i| i.id.clone())
        .collect();
    RefreshReport {
        scan: scan_items(discovery, cancel, emit),
        removed_ids,
        invalidated,
    }
}
