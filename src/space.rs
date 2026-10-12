//! 只读空间统计与有界树图布局；共享目录按规范路径去重。
use crate::{models::Item, platform::*};
use anyhow::Result;
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

pub const MAX_NODES: usize = 100_000;
pub const MAX_TILES: usize = 256;

#[derive(Clone, Debug)]
pub struct Node {
    pub name: String,
    pub path: Option<PathBuf>,
    pub bytes: u64,
    pub complete: bool,
    pub directory: bool,
    pub category: usize,
    pub owner: String,
    pub item_id: Option<String>,
    pub children: Vec<usize>,
    pub omitted: u64,
}

#[derive(Clone, Debug)]
pub struct Analysis {
    pub nodes: Vec<Node>,
    pub categories: [u64; 6],
    pub files: u64,
    pub skipped: u64,
    pub cancelled: bool,
}

#[derive(Clone, Debug)]
pub struct Tile {
    pub node: Option<usize>,
    pub bytes: u64,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub category: usize,
}

pub fn category(path: &Path) -> usize {
    // 越靠近叶子的类别优先，例如世界目录内部的数据包。
    path.components()
        .rev()
        .find_map(
            |c| match c.as_os_str().to_str()?.to_ascii_lowercase().as_str() {
                "saves" => Some(0),
                "resourcepacks" => Some(1),
                "mods" => Some(2),
                "shaderpacks" => Some(3),
                "datapacks" => Some(4),
                _ => None,
            },
        )
        .unwrap_or(5)
}

pub fn deduplicate_roots(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut paths = paths
        .iter()
        .map(|p| checked_path(p))
        .collect::<Result<Vec<_>>>()?;
    paths.sort_by_key(|p| (p.components().count(), path_key(p)));
    let mut roots: Vec<PathBuf> = Vec::new();
    for path in paths {
        if !roots.iter().any(|root| {
            let key = path_key(root);
            let candidate = path_key(&path);
            candidate == key
                || candidate.starts_with(&format!(
                    "{}{}",
                    key.trim_end_matches(std::path::MAIN_SEPARATOR),
                    std::path::MAIN_SEPARATOR
                ))
        }) {
            roots.push(path);
        }
    }
    Ok(roots)
}

/// 先枚举两层目录，再用最多四个后台线程统计互不重叠的子树。
pub fn analyze(
    paths: &[PathBuf],
    items: &[Item],
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64),
) -> Result<Analysis> {
    let mut jobs = Vec::new();
    let mut result = analyze_inner(
        paths,
        items,
        cancel,
        &mut progress,
        Some(&mut jobs),
        MAX_NODES,
    )?;
    if result.cancelled || jobs.is_empty() {
        return Ok(result);
    }
    let targets = jobs
        .iter()
        .map(|(i, p)| (path_key(p), *i))
        .collect::<std::collections::HashMap<_, _>>();
    if jobs.len() < 8 {
        for (_, path) in jobs {
            let base = result.files;
            let tree = analyze_inner(
                &[path],
                items,
                cancel,
                |n| progress(base + n),
                None,
                MAX_NODES.saturating_sub(result.nodes.len()).max(256),
            )?;
            merge(&mut result, tree, &targets);
            if result.cancelled {
                break;
            }
        }
    } else {
        use std::sync::{
            atomic::{AtomicU64, Ordering},
            mpsc,
        };
        enum Update {
            Progress(u64),
            Finished(Result<Analysis>),
        }
        let (sender, receiver) = mpsc::channel();
        let counted = AtomicU64::new(result.files);
        std::thread::scope(|scope| {
            for worker in 0..4 {
                let paths = jobs
                    .iter()
                    .enumerate()
                    .filter(|(n, _)| n % 4 == worker)
                    .map(|(_, (_, p))| p.clone())
                    .collect::<Vec<_>>();
                let sender = sender.clone();
                let counted = &counted;
                scope.spawn(move || {
                    let mut previous = 0;
                    let tree = analyze_inner(
                        &paths,
                        items,
                        cancel,
                        |n| {
                            let total =
                                counted.fetch_add(n - previous, Ordering::Relaxed) + n - previous;
                            previous = n;
                            let _ = sender.send(Update::Progress(total));
                        },
                        None,
                        MAX_NODES / 4,
                    );
                    let _ = sender.send(Update::Finished(tree));
                });
            }
            drop(sender);
            for update in receiver {
                match update {
                    Update::Progress(files) => progress(files),
                    Update::Finished(Ok(tree)) => merge(&mut result, tree, &targets),
                    Update::Finished(Err(_)) => {
                        result.skipped += 1;
                        result.nodes[0].complete = false;
                    }
                }
            }
        });
    }
    Ok(result)
}

// 子树已在各线程内累加完毕；合并只累加其外部祖先，不重复累加内部文件。
fn merge(
    result: &mut Analysis,
    tree: Analysis,
    targets: &std::collections::HashMap<String, usize>,
) {
    let mut parents = vec![0; result.nodes.len()];
    for (i, node) in result.nodes.iter().enumerate() {
        for child in &node.children {
            parents[*child] = i;
        }
    }
    let mut old_parents = vec![0; tree.nodes.len()];
    for (i, node) in tree.nodes.iter().enumerate() {
        for child in &node.children {
            old_parents[*child] = i;
        }
    }
    let mut remap = vec![0; tree.nodes.len()];
    for (i, node) in tree.nodes.iter().enumerate().skip(1) {
        if old_parents[i] == 0 {
            let Some(target) = node
                .path
                .as_ref()
                .and_then(|p| targets.get(&path_key(p)))
                .copied()
            else {
                continue;
            };
            remap[i] = target;
            let mut ancestor = parents[target];
            loop {
                result.nodes[ancestor].bytes =
                    result.nodes[ancestor].bytes.saturating_add(node.bytes);
                result.nodes[ancestor].complete &= node.complete;
                if ancestor == 0 {
                    break;
                }
                ancestor = parents[ancestor];
            }
            let target_node = &mut result.nodes[target];
            target_node.bytes = node.bytes;
            target_node.complete = node.complete;
            target_node.omitted = node.omitted;
        } else {
            let parent = remap[old_parents[i]];
            if result.nodes.len() < MAX_NODES {
                let target = result.nodes.len();
                remap[i] = target;
                let mut copied = node.clone();
                copied.children.clear();
                if copied.owner.is_empty() {
                    copied.owner = result.nodes[parent].owner.clone();
                }
                result.nodes.push(copied);
                result.nodes[parent].children.push(target);
            } else {
                remap[i] = parent;
                result.nodes[parent].omitted += 1;
            }
        }
    }
    result.files += tree.files;
    result.skipped += tree.skipped;
    result.cancelled |= tree.cancelled;
    result.nodes[0].complete &= tree.nodes[0].complete;
    for (total, bytes) in result.categories.iter_mut().zip(tree.categories) {
        *total = total.saturating_add(bytes);
    }
}

fn analyze_inner(
    paths: &[PathBuf],
    items: &[Item],
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64),
    mut split_jobs: Option<&mut Vec<(usize, PathBuf)>>,
    budget: usize,
) -> Result<Analysis> {
    let mut unreadable = 0;
    let accessible = paths
        .iter()
        .filter_map(|path| match checked_path(path) {
            Ok(path) => Some(path),
            Err(_) => {
                unreadable += 1;
                None
            }
        })
        .collect::<Vec<_>>();
    let roots = deduplicate_roots(&accessible)?;
    let mut result = Analysis {
        nodes: vec![Node {
            name: "全部来源".into(),
            path: None,
            bytes: 0,
            complete: unreadable == 0,
            directory: true,
            category: 5,
            owner: String::new(),
            item_id: None,
            children: Vec::new(),
            omitted: 0,
        }],
        categories: [0; 6],
        files: 0,
        skipped: unreadable,
        cancelled: false,
    };
    let item_paths = items
        .iter()
        .map(|i| (path_key(&i.path), i))
        .collect::<std::collections::HashMap<_, _>>();
    for root in roots {
        let mut parents = std::collections::HashMap::new();
        let mut depths = std::collections::HashMap::new();
        let interrupted = crate::walk::walk(&root, cancel, |event| {
            use crate::walk::Event;
            match event {
                Event::Directory { id, parent, path } => {
                    let depth = parent
                        .and_then(|p| depths.get(&p).copied())
                        .map_or(0, |d| d + 1);
                    depths.insert(id, depth);
                    let parent = parent.and_then(|p| parents.get(&p).copied()).unwrap_or(0);
                    if path
                        .file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with(".minecraft-explorer-"))
                    {
                        result.skipped += 1;
                        result.nodes[parent].complete = false;
                        parents.insert(id, parent);
                        return false;
                    }
                    let index = add_node(
                        &mut result,
                        path,
                        parent,
                        true,
                        &item_paths,
                        budget,
                        id == 0,
                    );
                    parents.insert(id, index);
                    if depth == 2
                        && index != parent
                        && let Some(jobs) = split_jobs.as_mut()
                    {
                        jobs.push((index, path.into()));
                        return false;
                    }
                }
                Event::File {
                    parent,
                    path,
                    bytes,
                } => {
                    let parent = parents[&parent];
                    if path
                        .file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with(".minecraft-explorer-"))
                    {
                        result.skipped += 1;
                        result.nodes[parent].complete = false;
                        return true;
                    }
                    let index =
                        add_node(&mut result, path, parent, false, &item_paths, budget, false);
                    result.files += 1;
                    result.nodes[index].bytes = result.nodes[index].bytes.saturating_add(bytes);
                    let category = category(path);
                    result.categories[category] = result.categories[category].saturating_add(bytes);
                    if result.files.is_multiple_of(1024) {
                        progress(result.files);
                    }
                }
                Event::Skipped { parent } => {
                    result.skipped += 1;
                    result.nodes[parents[&parent]].complete = false;
                }
                Event::End { id } => {
                    parents.remove(&id);
                    depths.remove(&id);
                }
            }
            true
        });
        match interrupted {
            Ok(true) => {
                result.cancelled = true;
                result.nodes[0].complete = false;
                break;
            }
            Ok(false) => (),
            Err(_) => {
                result.skipped += 1;
                result.nodes[0].complete = false;
            }
        }
    }
    // 逆序累加，每个真实文件的逻辑字节只进入总量一次。
    for index in (0..result.nodes.len()).rev() {
        let children = result.nodes[index].children.clone();
        for child in children {
            result.nodes[index].bytes = result.nodes[index]
                .bytes
                .saturating_add(result.nodes[child].bytes);
            result.nodes[index].complete &= result.nodes[child].complete;
        }
    }
    Ok(result)
}

fn add_node(
    result: &mut Analysis,
    path: &Path,
    parent: usize,
    directory: bool,
    items: &std::collections::HashMap<String, &Item>,
    budget: usize,
    force: bool,
) -> usize {
    // 为后续目录保留预算，避免一个大目录的文件耗尽全部钻取节点。
    let limit = if directory { budget } else { budget * 4 / 5 };
    if result.nodes.len() >= MAX_NODES || (!force && result.nodes.len() >= limit) {
        result.nodes[parent].omitted += 1;
        return parent;
    }
    let item = items.get(&path_key(path));
    let index = result.nodes.len();
    result.nodes.push(Node {
        name: item.map(|i| i.name.clone()).unwrap_or_else(|| {
            path.file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into_owned()
        }),
        path: Some(path.into()),
        bytes: 0,
        complete: true,
        directory,
        category: category(path),
        owner: item
            .map(|i| i.ownership.clone())
            .unwrap_or_else(|| result.nodes[parent].owner.clone()),
        item_id: item.map(|i| i.id.clone()),
        children: Vec::new(),
        omitted: 0,
    });
    result.nodes[parent].children.push(index);
    index
}

impl Analysis {
    pub fn layout(&self, node: usize, aspect: f32) -> Vec<Tile> {
        let Some(parent) = self.nodes.get(node) else {
            return Vec::new();
        };
        if parent.bytes == 0 {
            return Vec::new();
        }
        let mut children = parent
            .children
            .iter()
            .copied()
            .filter(|i| self.nodes[*i].bytes > 0)
            .collect::<Vec<_>>();
        children.sort_by_key(|i| std::cmp::Reverse(self.nodes[*i].bytes));
        let mut weights: Vec<(Option<usize>, u64, usize)> = Vec::new();
        let mut aggregated = parent
            .bytes
            .saturating_sub(children.iter().map(|i| self.nodes[*i].bytes).sum::<u64>());
        for child in children {
            let entry = &self.nodes[child];
            if weights.len() >= MAX_TILES - 1 || (entry.bytes as f64 / parent.bytes as f64) < 0.0005
            {
                aggregated = aggregated.saturating_add(entry.bytes);
            } else {
                weights.push((Some(child), entry.bytes, entry.category));
            }
        }
        if aggregated > 0 {
            weights.push((None, aggregated, 5));
        }
        let mut tiles = Vec::with_capacity(weights.len());
        split(&weights, 0., 0., aspect.clamp(0.25, 8.), 1., &mut tiles);
        for tile in &mut tiles {
            tile.x /= aspect.clamp(0.25, 8.);
            tile.width /= aspect.clamp(0.25, 8.);
        }
        tiles
    }
}

// 按权重二分，面积守恒；鼠标交互只读取缓存布局，不访问磁盘。
fn split(
    weights: &[(Option<usize>, u64, usize)],
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    output: &mut Vec<Tile>,
) {
    if weights.is_empty() {
        return;
    }
    if weights.len() == 1 {
        output.push(Tile {
            node: weights[0].0,
            bytes: weights[0].1,
            category: weights[0].2,
            x,
            y,
            width,
            height,
        });
        return;
    }
    let total = weights.iter().map(|v| v.1 as f64).sum::<f64>();
    let mut sum = 0.;
    let mut mid = 1;
    for (i, weight) in weights.iter().enumerate().take(weights.len() - 1) {
        sum += weight.1 as f64;
        mid = i + 1;
        if sum >= total / 2. {
            break;
        }
    }
    let ratio = (sum / total) as f32;
    if width >= height {
        split(&weights[..mid], x, y, width * ratio, height, output);
        split(
            &weights[mid..],
            x + width * ratio,
            y,
            width * (1. - ratio),
            height,
            output,
        );
    } else {
        split(&weights[..mid], x, y, width, height * ratio, output);
        split(
            &weights[mid..],
            x,
            y + height * ratio,
            width,
            height * (1. - ratio),
            output,
        );
    }
}
