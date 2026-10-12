//! 只读检查，使用与桌面程序相同的扫描器和索引。
use anyhow::{Context, Result, bail};
use minecraft_explorer::{
    index::Index,
    models::*,
    platform::*,
    scanner::{self, ScanEvent},
};
use std::{path::PathBuf, sync::atomic::AtomicBool, time::Instant};

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let supplied = PathBuf::from(
        args.next()
            .context("用法：inspect <Minecraft 根目录或 versions 目录> <独立缓存目录>")?,
    );
    let output = PathBuf::from(
        args.next()
            .context("请指定项目内独立缓存目录；不会写入 Minecraft 目录")?,
    );
    if args.next().is_some() {
        bail!("参数过多");
    }
    let supplied = checked_path(&supplied)?;
    let root = if supplied
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("versions"))
    {
        supplied
            .parent()
            .context("versions 没有父目录")?
            .to_path_buf()
    } else {
        supplied
    };
    let output = std::path::absolute(output)?;
    if path_key(&output).starts_with(&(path_key(&root) + "\\"))
        || path_key(&output) == path_key(&root)
    {
        bail!("缓存目录必须位于 Minecraft 根目录之外");
    }
    // 创建文件前验证现存祖先路径，避免输出目录的联接
    // 将缓存写入 Minecraft 或指定目录之外。
    let mut ancestor = output.as_path();
    while !ancestor.exists() {
        ancestor = ancestor.parent().context("缓存目录缺少可访问的父目录")?;
    }
    checked_path(ancestor)?;
    std::fs::create_dir_all(&output)?;
    let output = checked_path(&output)?;
    let settings = Settings {
        sources: vec![Source {
            path: root.clone(),
            kind: SourceKind::Root,
            associated_root: None,
        }],
        ..Default::default()
    };
    let mut db = Index::open(&output.join("index.db"))?;
    db.save_settings(&settings)?;
    let generation = now_ms();
    let started = Instant::now();
    let mut failure = None;
    let mut progress_at = Instant::now();
    let report = scanner::scan(&settings, &AtomicBool::new(false), |event| {
        let result = match event {
            ScanEvent::Discovered(items) => {
                eprintln!("发现 {} 个条目", items.len());
                db.discovered(&items, generation)
            }
            ScanEvent::Metadata(mut item) => {
                if let Ok(Some(old)) = db.get(&item.id) {
                    item.size = old.size;
                    item.size_complete = old.size_complete;
                }
                db.upsert(&item, generation)
            }
            ScanEvent::Updated(item) => db.upsert(&item, generation),
            ScanEvent::Progress {
                done,
                total,
                message,
            } => {
                if progress_at.elapsed().as_secs() >= 3 {
                    eprintln!("{done}/{total} {message}");
                    progress_at = Instant::now();
                }
                Ok(())
            }
            ScanEvent::Warning(warning) => {
                eprintln!("{warning}");
                Ok(())
            }
        };
        if let Err(e) = result {
            failure = Some(e);
        }
    });
    if let Some(e) = failure {
        return Err(e);
    }
    for source in &report.completed_sources {
        db.finish_source(source, generation)?;
    }
    let stats = db.stats()?;
    let mut samples = report
        .items
        .iter()
        .filter(|i| i.kind == ItemKind::World)
        .collect::<Vec<_>>();
    samples.sort_by_key(|i| std::cmp::Reverse(i.last_played_ms.or(i.modified_ms)));
    let summary = serde_json::json!({
        "root": display_path(&root), "cache": display_path(&output), "elapsed_seconds": started.elapsed().as_secs_f64(),
        "worlds": stats.worlds, "versions": stats.versions, "instances": stats.instances, "world_logical_bytes": stats.world_bytes,
        "resource_packs":stats.resource_packs,"mods":stats.mods,"shader_packs":stats.shader_packs,"data_packs":stats.data_packs,
        "worlds_with_icon":samples.iter().filter(|i|i.icon.is_some()).count(),
        "content_with_declared_preview":report.items.iter().filter(|i|i.content.as_ref().is_some_and(|m|m.preview_entry.is_some())).count(),
        "warnings": report.warnings, "incomplete_sizes": report.items.iter().filter(|i| !i.size_complete).count(),
        "worlds_with_nbt_time": samples.iter().filter(|i| i.last_played_ms.is_some()).count(),
        "recent_worlds": samples.iter().take(10).map(|i| serde_json::json!({"name":i.name,"path":display_path(&i.path),"owner":i.ownership,"bytes":i.size,"metadata":i.metadata_status})).collect::<Vec<_>>()
    });
    let json = serde_json::to_string_pretty(&summary)?;
    std::fs::write(output.join("report.json"), &json)?;
    println!("{json}");
    Ok(())
}
