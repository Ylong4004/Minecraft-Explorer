use minecraft_explorer::{
    discovery::base_item,
    duplicates,
    index::Index,
    models::*,
    platform::{checked_path, path_key},
    watch::Monitor,
};
use rusqlite::{Connection, params};
use std::{
    fs,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

fn item(path: &Path, kind: ItemKind) -> Item {
    let mut item = base_item(
        kind,
        path.into(),
        path.parent().unwrap().into(),
        "source".into(),
    );
    item.size = Some(1);
    item.size_complete = true;
    item
}

#[test]
fn annotations_survive_refresh_removal_and_restore() {
    let fixture = tempfile::tempdir().unwrap();
    let mut index = Index::open(&fixture.path().join("index.sqlite3")).unwrap();
    let mut original = item(&fixture.path().join("mod.jar"), ItemKind::Mod);
    index.upsert(&original, 1).unwrap();
    index
        .annotate(
            &original.id,
            true,
            &[" 常用 ".into(), "常用".into(), "CPU".into(), "cpu".into()],
        )
        .unwrap();
    index.record_opened(&original.id).unwrap();
    let opened = index.get(&original.id).unwrap().unwrap().last_opened_ms;
    original.favorite = false;
    original.tags = vec!["扫描不得覆盖此标签".into()];
    original.name = "扫描后名称".into();
    index
        .discovered(std::slice::from_ref(&original), 2)
        .unwrap();
    index.upsert(&original, 2).unwrap();
    let restored = index.get(&original.id).unwrap().unwrap();
    assert!(restored.favorite);
    assert_eq!(restored.tags, ["常用", "CPU"]);
    assert_eq!(restored.last_opened_ms, opened);
    index
        .remove_items(std::slice::from_ref(&original.id))
        .unwrap();
    assert!(index.get(&original.id).unwrap().is_none());
    assert!(index.tags().unwrap().is_empty());
    index.upsert(&original, 3).unwrap();
    assert!(index.get(&original.id).unwrap().unwrap().favorite);
    assert_eq!(index.tags().unwrap(), ["CPU", "常用"]);
}

#[test]
fn tags_exact_filters_and_search_escape_wildcards() {
    let fixture = tempfile::tempdir().unwrap();
    let index = Index::open(&fixture.path().join("index.sqlite3")).unwrap();
    let a = item(&fixture.path().join("a.jar"), ItemKind::Mod);
    let b = item(&fixture.path().join("b.jar"), ItemKind::Mod);
    index.upsert(&a, 1).unwrap();
    index.upsert(&b, 1).unwrap();
    index
        .annotate(&a.id, true, &["性能".into(), "50%_Ready".into()])
        .unwrap();
    index
        .annotate(&b.id, false, &["高性能".into(), "50xxReady".into()])
        .unwrap();
    let exact = index
        .query(&Query {
            tag: Some("性能".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(exact.len(), 1);
    assert_eq!(exact[0].id, a.id);
    assert_eq!(
        index
            .query(&Query {
                text: "%_".into(),
                ..Default::default()
            })
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        index
            .query(&Query {
                tag: Some("50%_ready".into()),
                ..Default::default()
            })
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        index
            .query(&Query {
                favorite_only: true,
                ..Default::default()
            })
            .unwrap()
            .len(),
        1
    );
    assert!(
        index
            .query(&Query {
                tag: Some("' OR 1=1 --".into()),
                ..Default::default()
            })
            .unwrap()
            .is_empty()
    );
    assert!(index.annotate("missing", true, &[]).is_err());
}

#[test]
fn recent_sort_and_annotation_edits_keep_open_time() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("index.sqlite3");
    let index = Index::open(&path).unwrap();
    let a = item(&fixture.path().join("a.jar"), ItemKind::Mod);
    let b = item(&fixture.path().join("b.jar"), ItemKind::Mod);
    let c = item(&fixture.path().join("c.jar"), ItemKind::Mod);
    for item in [&a, &b, &c] {
        index.upsert(item, 1).unwrap();
    }
    index.record_opened(&a.id).unwrap();
    index.record_opened(&b.id).unwrap();
    let sql = Connection::open(&path).unwrap();
    sql.execute(
        "UPDATE annotations SET last_opened_ms=100 WHERE item_id=?1",
        [&a.id],
    )
    .unwrap();
    sql.execute(
        "UPDATE annotations SET last_opened_ms=200 WHERE item_id=?1",
        [&b.id],
    )
    .unwrap();
    index.annotate(&b.id, true, &["收藏".into()]).unwrap();
    let recent = index
        .query(&Query {
            recent_only: true,
            sort: Sort::Recent,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        recent.iter().map(|i| &i.id).collect::<Vec<_>>(),
        [&b.id, &a.id]
    );
    assert_eq!(recent[0].last_opened_ms, Some(200));
    index.record_opened(&b.id).unwrap();
    assert!(index.get(&b.id).unwrap().unwrap().favorite);
    assert_eq!(index.get(&b.id).unwrap().unwrap().tags, ["收藏"]);
    assert_eq!(
        index
            .query(&Query {
                sort: Sort::Recent,
                ..Default::default()
            })
            .unwrap()
            .last()
            .unwrap()
            .id,
        c.id
    );
}

#[test]
fn trusted_metadata_search_and_new_kind_statistics() {
    let fixture = tempfile::tempdir().unwrap();
    let index = Index::open(&fixture.path().join("index.sqlite3")).unwrap();
    for (n, kind) in [
        ItemKind::Mod,
        ItemKind::ResourcePack,
        ItemKind::ShaderPack,
        ItemKind::DataPack,
    ]
    .into_iter()
    .enumerate()
    {
        let mut entry = item(&fixture.path().join(format!("arbitrary-{n}")), kind);
        if n == 0 {
            entry.content = Some(ContentMetadata {
                description: Some("可信配置中的性能描述".into()),
                declared_id: Some("declared_mod".into()),
                declared_version: Some("9.8.7".into()),
                ..Default::default()
            });
        }
        index.upsert(&entry, 1).unwrap();
    }
    for term in ["性能描述", "DECLARED_MOD", "9.8.7"] {
        assert_eq!(
            index
                .query(&Query {
                    text: term.into(),
                    ..Default::default()
                })
                .unwrap()
                .len(),
            1
        );
    }
    let stats = index.stats().unwrap();
    assert_eq!(
        (
            stats.resource_packs,
            stats.mods,
            stats.shader_packs,
            stats.data_packs
        ),
        (1, 1, 1, 1)
    );
}

#[test]
fn schema_one_migrates_without_losing_settings_or_old_item_json() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("old.sqlite3");
    let original = item(&fixture.path().join("old-world"), ItemKind::World);
    let mut json = serde_json::to_value(&original).unwrap();
    for field in ["favorite", "tags", "last_opened_ms", "content"] {
        json.as_object_mut().unwrap().remove(field);
    }
    let old = Connection::open(&path).unwrap();
    old.execute_batch("CREATE TABLE settings(id INTEGER PRIMARY KEY CHECK(id=1),json TEXT NOT NULL);
        CREATE TABLE items(id TEXT PRIMARY KEY,kind TEXT NOT NULL,source TEXT NOT NULL,game_dir TEXT,name TEXT NOT NULL,search TEXT NOT NULL,size INTEGER,time_ms INTEGER,ownership TEXT NOT NULL,available INTEGER NOT NULL,generation INTEGER NOT NULL,json TEXT NOT NULL);
        INSERT INTO settings VALUES(1,'{\"sources\":[],\"launchers\":[],\"mappings\":[],\"default_launcher\":null}'); PRAGMA user_version=1;").unwrap();
    old.execute("INSERT INTO items VALUES(?1,'world','source',NULL,'旧世界','旧世界',1,NULL,'旧来源',1,123,?2)", params![original.id, json.to_string()]).unwrap();
    drop(old);
    let index = Index::open(&path).unwrap();
    let settings = index.load_settings().unwrap();
    assert!(settings.watch_enabled);
    let restored = index.get(&original.id).unwrap().unwrap();
    assert!(!restored.favorite);
    assert!(restored.tags.is_empty());
    assert!(restored.content.is_none());
    assert_eq!(restored.path, original.path);
    assert_eq!(index.stats().unwrap().worlds, 1);
    let sql = Connection::open(&path).unwrap();
    assert_eq!(
        sql.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn future_schema_is_rejected_and_left_unchanged() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("future.sqlite3");
    let future = Connection::open(&path).unwrap();
    future.execute_batch("CREATE TABLE marker(value TEXT); INSERT INTO marker VALUES('future'); PRAGMA user_version=99;").unwrap();
    drop(future);
    assert!(Index::open(&path).is_err());
    let sql = Connection::open(&path).unwrap();
    assert_eq!(
        sql.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        99
    );
    assert_eq!(
        sql.query_row("SELECT value FROM marker", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "future"
    );
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name='items'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn duplicate_hashes_cover_whole_file_ignore_directories_and_deduplicate_paths() {
    let fixture = tempfile::tempdir().unwrap();
    let a = fixture.path().join("a.jar");
    let b = fixture.path().join("b.zip");
    let c = fixture.path().join("same-size-different.jar");
    let world = fixture.path().join("level.dat");
    let folder = fixture.path().join("unpacked");
    let mut bytes = vec![42_u8; 600_000];
    fs::write(&a, &bytes).unwrap();
    fs::write(&b, &bytes).unwrap();
    bytes[599_999] = 43;
    fs::write(&c, &bytes).unwrap();
    fs::write(&world, &bytes).unwrap();
    fs::create_dir(&folder).unwrap();
    let items = [
        item(&a, ItemKind::Mod),
        item(&b, ItemKind::ResourcePack),
        item(&c, ItemKind::Mod),
        item(&world, ItemKind::World),
        item(&folder, ItemKind::ResourcePack),
        item(&a, ItemKind::DataPack),
    ];
    let report = duplicates::find(&items, &AtomicBool::new(false), |_| {});
    assert!(!report.cancelled);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert_eq!(report.hashed_files, 3);
    assert_eq!(report.hashed_bytes, 1_800_000);
    assert_eq!(report.groups.len(), 1);
    assert_eq!(report.groups[0].size, 600_000);
    assert_eq!(report.groups[0].items.len(), 2);
    assert_eq!(
        report.groups[0].hash,
        blake3::hash(&fs::read(&a).unwrap()).to_hex().to_string()
    );
}

#[test]
fn duplicate_read_cancellation_stops_inside_large_file() {
    let fixture = tempfile::tempdir().unwrap();
    let a = fixture.path().join("a.jar");
    let b = fixture.path().join("b.jar");
    fs::write(&a, vec![1; 2_000_000]).unwrap();
    fs::write(&b, vec![1; 2_000_000]).unwrap();
    let cancel = AtomicBool::new(false);
    let report = duplicates::find(
        &[item(&a, ItemKind::Mod), item(&b, ItemKind::Mod)],
        &cancel,
        |status| {
            if status.bytes > 0 {
                cancel.store(true, Ordering::Relaxed);
            }
        },
    );
    assert!(report.cancelled);
    assert!(report.groups.is_empty());
    assert!(report.hashed_bytes > 0 && report.hashed_bytes < 2_000_000);
}

#[test]
fn changing_file_is_not_reported_as_equal() {
    let fixture = tempfile::tempdir().unwrap();
    let a = fixture.path().join("a.jar");
    let b = fixture.path().join("b.jar");
    fs::write(&a, vec![1; 900_000]).unwrap();
    fs::write(&b, vec![1; 900_000]).unwrap();
    let mut changed = false;
    let report = duplicates::find(
        &[item(&a, ItemKind::Mod), item(&b, ItemKind::Mod)],
        &AtomicBool::new(false),
        |status| {
            if !changed && status.bytes > 0 {
                fs::write(&a, [7_u8; 100]).unwrap();
                changed = true;
            }
        },
    );
    assert!(changed);
    assert!(report.groups.is_empty());
    assert_eq!(report.warnings.len(), 1);
    assert!(report.warnings[0].contains("变化"));
}

fn next_batch(monitor: &mut Monitor, timeout: Duration) -> minecraft_explorer::watch::WatchBatch {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(batch) = monitor.poll() {
            return batch;
        }
        assert!(
            Instant::now() < deadline,
            "native file notification did not arrive"
        );
        thread::sleep(Duration::from_millis(15));
    }
}

#[test]
fn native_watch_debounces_create_modify_rename_delete_and_reconfigure() {
    let fixture = tempfile::tempdir().unwrap();
    let root = checked_path(fixture.path()).unwrap();
    let nested = root.join("mods");
    fs::create_dir(&nested).unwrap();
    let other = tempfile::tempdir().unwrap();
    let mut monitor = Monitor::new(Duration::from_millis(100)).unwrap();
    assert!(
        monitor
            .reconfigure(&[nested.clone(), root.clone()])
            .unwrap()
            .is_empty()
    );
    assert_eq!(monitor.watched_paths().len(), 1);
    let a = nested.join("added.jar");
    fs::write(&a, b"first").unwrap();
    // 以相同根目录重新配置时，保留原生后端已排队的事件。
    assert!(
        monitor
            .reconfigure(std::slice::from_ref(&root))
            .unwrap()
            .is_empty()
    );
    let first = next_batch(&mut monitor, Duration::from_secs(5));
    assert!(!first.overflow);
    assert!(
        first
            .paths
            .iter()
            .any(|path| path_key(path) == path_key(&a))
    );
    assert!(monitor.poll().is_none());
    fs::write(&a, b"changed bytes").unwrap();
    let modified = next_batch(&mut monitor, Duration::from_secs(5));
    assert!(
        modified
            .paths
            .iter()
            .any(|path| path_key(path) == path_key(&a))
    );
    let b = nested.join("renamed.jar");
    fs::rename(&a, &b).unwrap();
    let renamed = next_batch(&mut monitor, Duration::from_secs(5));
    assert!(
        renamed
            .paths
            .iter()
            .any(|path| path_key(path) == path_key(&a))
    );
    assert!(
        renamed
            .paths
            .iter()
            .any(|path| path_key(path) == path_key(&b))
    );
    fs::remove_file(&b).unwrap();
    let deleted = next_batch(&mut monitor, Duration::from_secs(5));
    assert!(
        deleted
            .paths
            .iter()
            .any(|path| path_key(path) == path_key(&b))
    );
    assert!(
        monitor
            .reconfigure(&[other.path().into()])
            .unwrap()
            .is_empty()
    );
    assert!(next_batch(&mut monitor, Duration::from_secs(5)).overflow);
    fs::write(nested.join("unwatched.jar"), b"outside").unwrap();
    thread::sleep(Duration::from_millis(150));
    assert!(monitor.poll().is_none());
    let external = other.path().join("external.jar");
    fs::write(&external, b"tracked").unwrap();
    let batch = next_batch(&mut monitor, Duration::from_secs(5));
    assert!(
        batch
            .paths
            .iter()
            .any(|path| path_key(path) == path_key(&external))
    );
    assert!(monitor.reconfigure(&[]).unwrap().is_empty());
    assert!(monitor.watched_paths().is_empty());
}

#[test]
fn offline_watch_path_can_reconnect_without_losing_valid_roots() {
    let fixture = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    let missing = external.path().join("temporarily-offline");
    let desired = [fixture.path().to_path_buf(), missing.clone()];
    let mut monitor = Monitor::new(Duration::from_millis(50)).unwrap();
    assert_eq!(monitor.reconfigure(&desired).unwrap().len(), 1);
    assert!(monitor.needs_retry());
    assert_eq!(monitor.watched_paths().len(), 1);
    fs::create_dir(&missing).unwrap();
    assert!(monitor.reconfigure(&desired).unwrap().is_empty());
    assert!(!monitor.needs_retry());
    assert_eq!(monitor.watched_paths().len(), 2);
    assert!(next_batch(&mut monitor, Duration::from_secs(5)).overflow);
    let restored = missing.join("restored.jar");
    fs::write(&restored, b"newly available").unwrap();
    assert!(
        next_batch(&mut monitor, Duration::from_secs(5))
            .paths
            .iter()
            .any(|path| path_key(path) == path_key(&restored))
    );
}

#[test]
fn recovering_only_offline_root_requests_discovery_even_without_new_file_events() {
    let fixture = tempfile::tempdir().unwrap();
    let missing = fixture.path().join("temporarily-offline");
    let mut monitor = Monitor::new(Duration::from_millis(30)).unwrap();
    assert_eq!(
        monitor
            .reconfigure(std::slice::from_ref(&missing))
            .unwrap()
            .len(),
        1
    );
    assert!(monitor.watched_paths().is_empty());
    fs::create_dir(&missing).unwrap();
    fs::write(
        missing.join("already-restored.jar"),
        b"already here before watch",
    )
    .unwrap();
    assert!(monitor.reconfigure(&[missing]).unwrap().is_empty());
    assert!(next_batch(&mut monitor, Duration::from_secs(5)).overflow);
}

#[test]
fn native_watch_rebinds_when_root_is_deleted_and_recreated() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("replaceable-root");
    fs::create_dir(&root).unwrap();
    let mut monitor = Monitor::new(Duration::from_millis(30)).unwrap();
    assert!(
        monitor
            .reconfigure(std::slice::from_ref(&root))
            .unwrap()
            .is_empty()
    );
    fs::remove_dir(&root).unwrap();
    fs::create_dir(&root).unwrap();
    assert!(next_batch(&mut monitor, Duration::from_secs(8)).overflow);
    assert!(monitor.needs_retry());
    assert!(
        monitor
            .reconfigure(std::slice::from_ref(&root))
            .unwrap()
            .is_empty()
    );
    assert!(!monitor.needs_retry());
    assert!(next_batch(&mut monitor, Duration::from_secs(5)).overflow);
    let file = root.join("new-directory-object.jar");
    fs::write(&file, b"new subscription").unwrap();
    assert!(
        next_batch(&mut monitor, Duration::from_secs(5))
            .paths
            .iter()
            .any(|path| path_key(path) == path_key(&file))
    );
}
