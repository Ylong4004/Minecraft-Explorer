//! 流式只读遍历：复用 Windows 枚举元数据，目录句柄保护当前祖先链。
use crate::platform::{checked_path, is_link};
use anyhow::{Result, bail};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

pub enum Event<'a> {
    Directory {
        id: usize,
        parent: Option<usize>,
        path: &'a Path,
    },
    File {
        parent: usize,
        path: &'a Path,
        bytes: u64,
    },
    End {
        id: usize,
    },
    Skipped {
        parent: usize,
    },
}

struct Frame {
    id: usize,
    path: PathBuf,
    entries: fs::ReadDir,
    _guard: fs::File,
}

fn directory_guard(path: &Path) -> Result<fs::File> {
    #[cfg(windows)]
    let file = {
        use std::os::windows::fs::OpenOptionsExt;
        // 只申请目录枚举和属性权限；允许正常读写，暂时拒绝目录改名/删除。
        // OPEN_REPARSE_POINT 保证检查的是本级目录，而不是联接目标。
        fs::OpenOptions::new()
            .access_mode(0x81)
            .share_mode(3)
            .custom_flags(0x02000000 | 0x00200000)
            .open(path)?
    };
    #[cfg(not(windows))]
    let file = fs::File::open(checked_path(path)?)?;
    let metadata = file.metadata()?;
    if !metadata.is_dir() || is_link(&metadata) {
        bail!("目录不可访问或已经变为链接：{}", path.display());
    }
    Ok(file)
}

/// 返回是否取消；Directory 回调返回 false 可以复用缓存而跳过子树。
/// 同时持有活动目录的祖先句柄，避免反复探测整条路径，也避免祖先被替换。
pub fn walk(
    path: &Path,
    cancel: &AtomicBool,
    mut visit: impl FnMut(Event<'_>) -> bool,
) -> Result<bool> {
    let path = checked_path(path)?;
    let mut ancestors = Vec::new();
    // 从卷根往下持有句柄，每一级打开时上一级已经受到保护。
    #[cfg(windows)]
    {
        let mut chain = path.ancestors().collect::<Vec<_>>();
        chain.reverse();
        for ancestor in chain.into_iter().filter(|p| *p != path) {
            ancestors.push(directory_guard(ancestor)?);
        }
    }
    #[cfg(not(windows))]
    let _ = &mut ancestors as &mut Vec<fs::File>;
    let guard = directory_guard(&path)?;
    if cancel.load(Ordering::Relaxed) {
        return Ok(true);
    }
    if !visit(Event::Directory {
        id: 0,
        parent: None,
        path: &path,
    }) {
        visit(Event::End { id: 0 });
        return Ok(false);
    }
    let entries = match fs::read_dir(&path) {
        Ok(entries) => entries,
        Err(_) => {
            visit(Event::Skipped { parent: 0 });
            visit(Event::End { id: 0 });
            return Ok(false);
        }
    };
    let mut stack = vec![Frame {
        id: 0,
        path,
        entries,
        _guard: guard,
    }];
    let mut next_id = 1;
    while !stack.is_empty() {
        if cancel.load(Ordering::Relaxed) {
            return Ok(true);
        }
        let frame = stack.last_mut().unwrap();
        let parent = frame.id;
        let Some(entry) = frame.entries.next() else {
            visit(Event::End { id: parent });
            stack.pop();
            continue;
        };
        let Ok(entry) = entry else {
            visit(Event::Skipped { parent });
            continue;
        };
        let Ok(metadata) = entry.metadata() else {
            visit(Event::Skipped { parent });
            continue;
        };
        if is_link(&metadata) {
            visit(Event::Skipped { parent });
            continue;
        }
        let child = frame.path.join(entry.file_name());
        if metadata.is_file() {
            visit(Event::File {
                parent,
                path: &child,
                bytes: metadata.len(),
            });
        } else if metadata.is_dir() {
            if stack.len() >= 1024 {
                visit(Event::Skipped { parent });
                continue;
            }
            let Ok(guard) = directory_guard(&child) else {
                visit(Event::Skipped { parent });
                continue;
            };
            let id = next_id;
            next_id += 1;
            if !visit(Event::Directory {
                id,
                parent: Some(parent),
                path: &child,
            }) {
                visit(Event::End { id });
                continue;
            }
            match fs::read_dir(&child) {
                Ok(entries) => stack.push(Frame {
                    id,
                    path: child,
                    entries,
                    _guard: guard,
                }),
                Err(_) => {
                    visit(Event::Skipped { parent: id });
                    visit(Event::End { id });
                }
            }
        }
    }
    Ok(false)
}
