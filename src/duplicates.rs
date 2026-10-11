//! Read-only duplicate candidates. Equality refers to complete, stable file bytes only;
//! no files or directories are renamed, deleted, extracted, or otherwise modified.
use crate::{
    models::{Item, ItemKind},
    platform::{checked_path, is_link, path_key},
};
use anyhow::{Context, Result, bail};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File},
    io::Read,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Debug)]
pub struct DuplicateGroup {
    pub size: u64,
    pub hash: String,
    pub items: Vec<Item>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DuplicateProgress {
    pub done: u64,
    pub total: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug, Default)]
pub struct DuplicateReport {
    pub groups: Vec<DuplicateGroup>,
    pub warnings: Vec<String>,
    pub cancelled: bool,
    pub hashed_files: u64,
    pub hashed_bytes: u64,
}

pub fn find(
    items: &[Item],
    cancel: &AtomicBool,
    mut progress: impl FnMut(DuplicateProgress),
) -> DuplicateReport {
    let mut report = DuplicateReport::default();
    let mut sizes: BTreeMap<u64, Vec<Item>> = BTreeMap::new();
    let mut paths = HashSet::new();
    for item in items {
        if cancel.load(Ordering::Relaxed) {
            report.cancelled = true;
            return report;
        }
        if !item.available
            || !matches!(
                item.kind,
                ItemKind::ResourcePack | ItemKind::Mod | ItemKind::ShaderPack | ItemKind::DataPack
            )
        {
            continue;
        }
        let candidate = (|| -> Result<Option<u64>> {
            let checked = checked_path(&item.path)?;
            let metadata = fs::symlink_metadata(&checked)?;
            if !metadata.is_file() || is_link(&metadata) {
                return Ok(None);
            }
            if !paths.insert(path_key(&checked)) {
                return Ok(None);
            }
            Ok(Some(metadata.len()))
        })();
        match candidate {
            Ok(Some(size)) => sizes.entry(size).or_default().push(item.clone()),
            Ok(None) => {}
            Err(error) => report
                .warnings
                .push(format!("{}：{error}", item.path.display())),
        }
    }
    sizes.retain(|_, entries| entries.len() > 1);
    let total = sizes.values().map(|entries| entries.len() as u64).sum();
    let mut status = DuplicateProgress {
        total,
        ..Default::default()
    };
    progress(status);
    let mut equal: BTreeMap<(u64, String), Vec<Item>> = BTreeMap::new();
    // One file and one 256 KiB buffer at a time, independently of candidate count.
    let mut buffer = vec![0_u8; 256 * 1024];
    for (size, entries) in sizes {
        for item in entries {
            if cancel.load(Ordering::Relaxed) {
                report.cancelled = true;
                break;
            }
            match stable_hash(&item.path, size, cancel, &mut buffer, |bytes| {
                status.bytes += bytes;
                progress(status);
            }) {
                Ok(Some(hash)) => {
                    report.hashed_files += 1;
                    equal.entry((size, hash)).or_default().push(item);
                }
                Ok(None) => {
                    report.cancelled = true;
                    break;
                }
                Err(error) => report
                    .warnings
                    .push(format!("{}：{error}", item.path.display())),
            }
            status.done += 1;
            progress(status);
        }
        if report.cancelled {
            break;
        }
    }
    report.hashed_bytes = status.bytes;
    report.groups = equal
        .into_iter()
        .filter(|(_, entries)| entries.len() > 1)
        .map(|((size, hash), items)| DuplicateGroup { size, hash, items })
        .collect();
    report
}

fn stable_hash(
    path: &Path,
    expected_size: u64,
    cancel: &AtomicBool,
    buffer: &mut [u8],
    mut progress: impl FnMut(u64),
) -> Result<Option<String>> {
    let checked = checked_path(path)?;
    let before = fs::symlink_metadata(&checked)?;
    if !before.is_file() || is_link(&before) || before.len() != expected_size {
        bail!("候选文件类型或大小已变化，跳过");
    }
    let modified = before.modified().context("无法确认文件修改时间，跳过")?;
    let mut file = File::open(&checked)?;
    let opened = file.metadata()?;
    if opened.len() != expected_size || opened.modified()? != modified {
        bail!("文件在打开时已变化，跳过");
    }
    let mut hasher = blake3::Hasher::new();
    let mut bytes = 0_u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        // Re-check every component before each read; reparse points never become a hash source.
        if path_key(&checked_path(path)?) != path_key(&checked) {
            bail!("读取期间路径已变化，跳过");
        }
        let count = file.read(buffer)?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        if bytes > expected_size {
            bail!("读取期间文件大小已变化，跳过");
        }
        hasher.update(&buffer[..count]);
        progress(count as u64);
    }
    let after_path = checked_path(path)?;
    let after = fs::symlink_metadata(&after_path)?;
    let after_handle = file.metadata()?;
    if bytes != expected_size
        || !after.is_file()
        || is_link(&after)
        || path_key(&after_path) != path_key(&checked)
        || after.len() != expected_size
        || after.modified()? != modified
        || after_handle.len() != expected_size
        || after_handle.modified()? != modified
    {
        bail!("读取期间文件已变化，无法确认重复，跳过");
    }
    Ok(Some(hasher.finalize().to_hex().to_string()))
}
