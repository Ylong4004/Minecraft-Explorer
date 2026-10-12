use minecraft_explorer::{models::*, platform::checked_path, scanner::SizeCache, space, walk};
use std::{
    fs,
    sync::atomic::{AtomicBool, Ordering},
};

#[test]
fn instance_world_and_pack_reuse_one_subtree_enumeration() {
    let fixture = tempfile::tempdir().unwrap();
    let instance = fixture.path().join("instance");
    let world = instance.join("saves/world");
    let pack = world.join("datapacks/pack");
    fs::create_dir_all(pack.join("data")).unwrap();
    fs::write(instance.join("core.json"), [0; 13]).unwrap();
    fs::write(instance.join("core.jar"), [0; 17]).unwrap();
    fs::write(world.join("level.dat"), [0; 19]).unwrap();
    fs::write(pack.join("pack.mcmeta"), [0; 23]).unwrap();
    fs::write(pack.join("data/file"), [0; 29]).unwrap();
    let cancel = AtomicBool::new(false);
    let mut cache = SizeCache::default();
    assert_eq!(
        cache
            .size(
                &checked_path(&instance).unwrap(),
                &ItemKind::Instance,
                false,
                &cancel,
                |_| {}
            )
            .unwrap(),
        (71, true)
    );
    let visits = cache.enumerated_directories;
    assert_eq!(
        cache
            .size(
                &checked_path(&world).unwrap(),
                &ItemKind::World,
                false,
                &cancel,
                |_| {}
            )
            .unwrap(),
        (71, true)
    );
    assert_eq!(
        cache
            .size(
                &checked_path(&pack).unwrap(),
                &ItemKind::DataPack,
                false,
                &cancel,
                |_| {}
            )
            .unwrap(),
        (52, true)
    );
    assert_eq!(cache.enumerated_directories, visits);
    assert_eq!(
        cache
            .size(
                &checked_path(&instance).unwrap(),
                &ItemKind::Version,
                false,
                &cancel,
                |_| {}
            )
            .unwrap(),
        (30, true)
    );
}

#[test]
fn parallel_subtrees_preserve_bytes_categories_and_drill_down() {
    let fixture = tempfile::tempdir().unwrap();
    for n in 0..24 {
        let path = fixture
            .path()
            .join(format!("versions/v{n}/saves/world/region"));
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("r.mca"), vec![0; n + 1]).unwrap();
    }
    let mut owner = minecraft_explorer::discovery::base_item(
        ItemKind::Instance,
        checked_path(fixture.path()).unwrap(),
        checked_path(fixture.path()).unwrap(),
        "fixture".into(),
    );
    owner.ownership = "共享目录测试".into();
    let tree = space::analyze(
        &[fixture.path().into(), fixture.path().join("versions")],
        &[owner],
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!(tree.files, 24);
    assert_eq!(tree.nodes[0].bytes, 300);
    assert_eq!(tree.categories, [300, 0, 0, 0, 0, 0]);
    assert!(tree.nodes[0].complete);
    assert!(
        tree.nodes
            .iter()
            .skip(1)
            .all(|node| node.owner == "共享目录测试")
    );
    for (index, node) in tree.nodes.iter().enumerate() {
        assert_eq!(
            tree.layout(index, 1.8).iter().map(|t| t.bytes).sum::<u64>(),
            node.bytes
        );
    }
}

#[test]
fn parallel_cancellation_keeps_an_honest_lower_bound() {
    let fixture = tempfile::tempdir().unwrap();
    for n in 0..8 {
        let path = fixture.path().join(format!("versions/v{n}"));
        fs::create_dir_all(&path).unwrap();
        for file in 0..1100 {
            fs::write(path.join(format!("f{file}")), [0; 3]).unwrap();
        }
    }
    let cancel = AtomicBool::new(false);
    let tree = space::analyze(&[fixture.path().into()], &[], &cancel, |_| {
        cancel.store(true, Ordering::Relaxed)
    })
    .unwrap();
    assert!(tree.cancelled && !tree.nodes[0].complete);
    assert!(tree.files < 8800);
    assert_eq!(tree.nodes[0].bytes, tree.files * 3);
    assert_eq!(tree.categories.iter().sum::<u64>(), tree.files * 3);
}

#[cfg(windows)]
#[test]
fn active_directory_chain_cannot_be_replaced_during_walk() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("root");
    fs::create_dir_all(root.join("child")).unwrap();
    fs::write(root.join("child/file"), [0; 7]).unwrap();
    let mut seen = 0;
    walk::walk(&root, &AtomicBool::new(false), |event| {
        if let walk::Event::Directory { path, .. } = event {
            assert!(fs::rename(path, path.with_extension("moved")).is_err());
            seen += 1;
        }
        true
    })
    .unwrap();
    assert_eq!(seen, 2);
    assert!(fs::rename(&root, fixture.path().join("moved")).is_ok());
}

#[test]
fn parallel_package_cancel_discards_inflight_updates_and_retry_reads_all_metadata() {
    use minecraft_explorer::scanner::{self, ScanEvent};
    use std::io::Write;
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("Minecraft");
    fs::create_dir_all(root.join("mods")).unwrap();
    for n in 0..64 {
        let file = fs::File::create(root.join(format!("mods/mod{n}.jar"))).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("fabric.mod.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        write!(zip, "{{\"schemaVersion\":1,\"id\":\"mod{n}\",\"version\":\"1.0\",\"name\":\"Declared {n}\"}}").unwrap();
        zip.finish().unwrap();
    }
    let settings = Settings {
        sources: vec![Source {
            path: checked_path(&root).unwrap(),
            kind: SourceKind::Root,
            associated_root: None,
        }],
        ..Default::default()
    };
    let cancel = AtomicBool::new(false);
    let partial = scanner::scan(&settings, &cancel, |event| {
        if let ScanEvent::Updated(item) = event
            && item.kind == ItemKind::Mod
        {
            cancel.store(true, Ordering::Relaxed);
        }
    });
    assert!(partial.cancelled);
    assert_eq!(
        partial
            .items
            .iter()
            .filter(|i| i.kind == ItemKind::Mod)
            .count(),
        1
    );
    cancel.store(false, Ordering::Relaxed);
    let complete = scanner::scan(&settings, &cancel, |_| {});
    assert!(!complete.cancelled);
    let mods = complete
        .items
        .iter()
        .filter(|i| i.kind == ItemKind::Mod)
        .collect::<Vec<_>>();
    assert_eq!(mods.len(), 64);
    assert!(
        mods.iter()
            .all(|i| i.name.starts_with("Declared ") && i.size.is_some() && i.size_complete)
    );
    assert_eq!(
        mods.iter()
            .map(|i| i.content.as_ref().unwrap().declared_id.clone().unwrap())
            .collect::<std::collections::HashSet<_>>()
            .len(),
        64
    );
}
