use minecraft_explorer::{
    discovery::base_item, models::ItemKind, platform::checked_path, space::*,
};
use std::{
    fs,
    sync::atomic::{AtomicBool, Ordering},
};

#[test]
fn overlapping_sources_and_shared_content_count_once() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("Minecraft 中文 & 空格");
    let world = root.join("saves/共享世界");
    fs::create_dir_all(&world).unwrap();
    fs::write(world.join("level.dat"), [0u8; 17]).unwrap();
    fs::write(root.join("other.bin"), [0u8; 23]).unwrap();
    let item = base_item(
        ItemKind::World,
        checked_path(&world).unwrap(),
        checked_path(&root).unwrap(),
        "fixture".into(),
    );
    let tree = analyze(
        &[world.clone(), root.clone(), root],
        std::slice::from_ref(&item),
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!(tree.nodes[0].bytes, 40);
    assert_eq!(tree.files, 2);
    assert_eq!(tree.categories, [17, 0, 0, 0, 0, 23]);
    assert!(tree.nodes[0].complete);
    let node = tree
        .nodes
        .iter()
        .find(|n| n.item_id.as_ref() == Some(&item.id))
        .unwrap();
    assert_eq!(node.bytes, 17);
    assert_eq!(node.name, item.name);
}

#[test]
fn external_game_directories_are_included_and_category_is_nearest() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    fs::create_dir_all(root.join("root/mods")).unwrap();
    fs::create_dir_all(root.join("external/saves/map/datapacks/pack/data")).unwrap();
    fs::write(root.join("root/mods/test.jar"), [0; 7]).unwrap();
    fs::write(
        root.join("external/saves/map/datapacks/pack/data/test.json"),
        [0; 11],
    )
    .unwrap();
    let tree = analyze(
        &[root.join("root"), root.join("external")],
        &[],
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!(tree.nodes[0].bytes, 18);
    assert_eq!(tree.categories, [0, 0, 7, 0, 11, 0]);
}

#[test]
fn cancelled_analysis_returns_a_lower_bound_without_overcounting() {
    let fixture = tempfile::tempdir().unwrap();
    for i in 0..1100 {
        fs::write(fixture.path().join(format!("file-{i}")), [0; 3]).unwrap();
    }
    let cancel = AtomicBool::new(false);
    let tree = analyze(&[fixture.path().into()], &[], &cancel, |_| {
        cancel.store(true, Ordering::Relaxed)
    })
    .unwrap();
    assert!(tree.cancelled);
    assert!(!tree.nodes[0].complete);
    assert_eq!(tree.files, 1024);
    assert_eq!(tree.nodes[0].bytes, 3072);
    assert_eq!(tree.categories.iter().sum::<u64>(), 3072);
}

#[test]
fn layout_preserves_areas_and_bounds_small_tile_controls() {
    let fixture = tempfile::tempdir().unwrap();
    for i in 0..1000 {
        fs::write(fixture.path().join(format!("file-{i}")), vec![0; i + 1]).unwrap();
    }
    let tree = analyze(
        &[fixture.path().into()],
        &[],
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    let directory = tree.nodes[0].children[0];
    for aspect in [0.3, 1., 2., 7.] {
        let tiles = tree.layout(directory, aspect);
        assert!(tiles.len() <= MAX_TILES);
        assert!(tiles.iter().any(|t| t.node.is_none()));
        assert_eq!(
            tiles.iter().map(|t| t.bytes).sum::<u64>(),
            tree.nodes[directory].bytes
        );
        let area = tiles.iter().map(|t| t.width * t.height).sum::<f32>();
        assert!((area - 1.).abs() < 0.0001);
        for tile in &tiles {
            assert!(
                tile.x >= 0.
                    && tile.y >= 0.
                    && tile.x + tile.width <= 1.00001
                    && tile.y + tile.height <= 1.00001
            );
            assert!(
                ((tile.width * tile.height) as f64
                    - tile.bytes as f64 / tree.nodes[directory].bytes as f64)
                    .abs()
                    < 0.00001
            );
        }
        for (i, a) in tiles.iter().enumerate() {
            for b in &tiles[i + 1..] {
                assert!(
                    a.x + a.width <= b.x + 0.00001
                        || b.x + b.width <= a.x + 0.00001
                        || a.y + a.height <= b.y + 0.00001
                        || b.y + b.height <= a.y + 0.00001
                );
            }
        }
    }
}

#[test]
fn empty_directory_and_zero_files_do_not_invent_areas() {
    let fixture = tempfile::tempdir().unwrap();
    fs::write(fixture.path().join("empty"), []).unwrap();
    let tree = analyze(
        &[fixture.path().into()],
        &[],
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert!(tree.layout(0, 1.8).is_empty());
    assert!(tree.layout(usize::MAX, 1.).is_empty());
    assert_eq!(tree.files, 1);
}

#[cfg(windows)]
#[test]
fn junctions_are_skipped_and_totals_are_marked_incomplete() {
    use std::process::Command;
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("root");
    let outside = fixture.path().join("outside");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret.bin"), [0; 99]).unwrap();
    assert!(
        Command::new("cmd.exe")
            .args(["/C", "mklink", "/J"])
            .arg(root.join("link"))
            .arg(outside)
            .output()
            .unwrap()
            .status
            .success()
    );
    let tree = analyze(&[root], &[], &AtomicBool::new(false), |_| {}).unwrap();
    assert_eq!(tree.nodes[0].bytes, 0);
    assert_eq!(tree.skipped, 1);
    assert!(!tree.nodes[0].complete);
}

#[test]
fn offline_source_does_not_hide_healthy_roots() {
    let fixture = tempfile::tempdir().unwrap();
    fs::write(fixture.path().join("file"), [0; 19]).unwrap();
    let tree = analyze(
        &[fixture.path().join("offline"), fixture.path().into()],
        &[],
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!(tree.nodes[0].bytes, 19);
    assert_eq!(tree.skipped, 1);
    assert!(!tree.nodes[0].complete);
}
