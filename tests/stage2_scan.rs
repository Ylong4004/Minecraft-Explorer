use minecraft_explorer::{
    index::Index,
    models::*,
    platform::{checked_path, modified_ms, path_key},
    scanner::{self, RefreshReport, ScanEvent, ScanReport},
};
use serde::Serialize;
use std::{
    fs,
    io::Write,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, UNIX_EPOCH},
};

fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn root(path: &Path) -> Source {
    Source {
        path: checked_path(path).unwrap(),
        kind: SourceKind::Root,
        associated_root: None,
    }
}

fn settings(path: &Path) -> Settings {
    Settings {
        sources: vec![root(path)],
        ..Default::default()
    }
}

#[derive(Serialize)]
struct Nbt<'a> {
    #[serde(rename = "Data")]
    data: NbtData<'a>,
}

#[derive(Serialize)]
struct NbtData<'a> {
    #[serde(rename = "LevelName")]
    name: &'a str,
    #[serde(rename = "LastPlayed")]
    last_played: i64,
    #[serde(rename = "Version")]
    version: NbtVersion,
}

#[derive(Serialize)]
struct NbtVersion {
    #[serde(rename = "Name")]
    name: &'static str,
}

fn world(path: &Path, name: &str, payload_size: usize) {
    let nbt = fastnbt::to_bytes(&Nbt {
        data: NbtData {
            name,
            last_played: 1_700_000_000_123,
            version: NbtVersion { name: "1.21.1" },
        },
    })
    .unwrap();
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&nbt).unwrap();
    write(&path.join("level.dat"), gzip.finish().unwrap());
    write(&path.join("region/r.0.0.mca"), vec![3_u8; payload_size]);
}

fn version(path: &Path, id: &str) {
    let folder = path.file_name().unwrap().to_string_lossy();
    write(
        &path.join(format!("{folder}.json")),
        serde_json::to_vec(&serde_json::json!({
            "id": id,
            "inheritsFrom": "1.21.1",
            "libraries": [{"name": "net.fabricmc:fabric-loader:0.16.0"}]
        }))
        .unwrap(),
    );
    write(&path.join(format!("{folder}.jar")), b"version core fixture");
}

fn pack_metadata(description: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "pack": {"description": description, "pack_format": 34}
    }))
    .unwrap()
}

fn directory_pack(path: &Path, description: &str, payload: &[u8]) {
    write(&path.join("pack.mcmeta"), pack_metadata(description));
    write(&path.join("data/example/payload.bin"), payload);
}

fn archive(path: &Path, entries: &[(&str, &[u8])]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut writer = zip::ZipWriter::new(fs::File::create(path).unwrap());
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in entries {
        writer.start_file(*name, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
}

fn mod_jar(path: &Path, name: &str, payload_size: usize) {
    let metadata = serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 1,
        "id": "fixture_mod",
        "version": "1.0.0",
        "name": name,
        "depends": {"minecraft": "1.21.x"}
    }))
    .unwrap();
    archive(
        path,
        &[
            ("fabric.mod.json", &metadata),
            ("assets/example/payload.bin", &vec![7_u8; payload_size]),
        ],
    );
}

fn scan(settings: &Settings) -> ScanReport {
    scanner::scan(settings, &AtomicBool::new(false), |_| {})
}

fn at<'a>(items: &'a [Item], path: &Path) -> &'a Item {
    items
        .iter()
        .find(|item| path_key(&item.path) == path_key(path))
        .unwrap_or_else(|| panic!("missing scanned path: {}", path.display()))
}

fn cache(index: &Index, report: &ScanReport) {
    for item in &report.items {
        index.upsert(item, 1).unwrap();
    }
}

fn apply_refresh(index: &Index, refresh: &RefreshReport, generation: i64) {
    assert!(
        !refresh.scan.cancelled,
        "cancelled reports are not cleanup plans"
    );
    for item in &refresh.scan.items {
        index.upsert(item, generation).unwrap();
    }
    index.remove_items(&refresh.removed_ids).unwrap();
    for item in &refresh.invalidated {
        index.upsert(item, generation).unwrap();
    }
}

#[test]
fn shared_directory_content_is_indexed_once_and_does_not_guess_a_version() {
    let fixture = tempfile::tempdir().unwrap();
    let game = fixture.path().join("Minecraft 共享");
    version(&game.join("versions/one"), "one-id");
    version(&game.join("versions/two"), "two-id");
    world(&game.join("saves/world"), "共享世界", 31);
    directory_pack(&game.join("resourcepacks/resources"), "资源", &[1; 17]);
    mod_jar(&game.join("mods/example.jar"), "共享模组", 29);
    archive(
        &game.join("shaderpacks/shader.zip"),
        &[("shaders/test.vsh", b"void main(){}")],
    );
    directory_pack(&game.join("saves/world/datapacks/data"), "数据", &[2; 19]);
    write(
        &game.join("launcher_profiles.json"),
        serde_json::to_vec(&serde_json::json!({
            "profiles": {
                "first": {"name": "别名一", "gameDir": game},
                "second": {"name": "别名二", "gameDir": game}
            }
        }))
        .unwrap(),
    );
    let mut settings = settings(&game);
    settings.sources.push(Source {
        path: checked_path(&game).unwrap(),
        kind: SourceKind::Custom,
        associated_root: Some(checked_path(&game).unwrap()),
    });
    let report = scan(&settings);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert_eq!(
        report
            .items
            .iter()
            .filter(|item| item.kind == ItemKind::Version)
            .count(),
        2
    );
    for kind in [
        ItemKind::Instance,
        ItemKind::World,
        ItemKind::ResourcePack,
        ItemKind::Mod,
        ItemKind::ShaderPack,
        ItemKind::DataPack,
    ] {
        let items: Vec<_> = report
            .items
            .iter()
            .filter(|item| item.kind == kind)
            .collect();
        assert_eq!(items.len(), 1, "unexpected duplicate {:?}", kind);
        assert_eq!(
            items[0].game_dir.as_ref(),
            Some(&checked_path(&game).unwrap())
        );
        assert!(
            items[0].version_id.is_none(),
            "shared data must not be assigned to one arbitrary version"
        );
    }
}

#[test]
fn world_data_packs_keep_their_physical_world_and_isolated_instance_owner() {
    let fixture = tempfile::tempdir().unwrap();
    let game = fixture.path().join("Minecraft");
    let first = game.join("versions/first");
    let second = game.join("versions/second");
    for (path, id, name) in [
        (&first, "first-id", "世界甲"),
        (&second, "second-id", "世界乙"),
    ] {
        version(path, id);
        world(&path.join("saves/world"), name, 27);
        directory_pack(&path.join("saves/world/datapacks/data"), name, &[4; 11]);
    }
    let report = scan(&settings(&game));
    assert_eq!(
        report
            .items
            .iter()
            .filter(|item| item.kind == ItemKind::DataPack)
            .count(),
        2
    );
    for (path, id, name) in [
        (&first, "first-id", "世界甲"),
        (&second, "second-id", "世界乙"),
    ] {
        let world = at(&report.items, &path.join("saves/world"));
        assert_eq!(world.name, name);
        let data = at(&report.items, &path.join("saves/world/datapacks/data"));
        assert_eq!(data.kind, ItemKind::DataPack);
        assert_eq!(data.root, checked_path(&game).unwrap());
        assert_eq!(data.game_dir.as_ref(), Some(&checked_path(path).unwrap()));
        assert_eq!(data.version_id.as_deref(), Some(id));
        assert!(data.ownership.contains("世界 world"));
        assert_eq!(
            data.content.as_ref().unwrap().description.as_deref(),
            Some(name)
        );
    }
}

#[test]
fn unpacked_packs_and_archive_files_use_actual_logical_file_sizes() {
    let fixture = tempfile::tempdir().unwrap();
    let game = fixture.path().join("Minecraft");
    let directory = game.join("resourcepacks/directory");
    let zip = game.join("resourcepacks/archive.zip");
    let jar = game.join("mods/example.jar");
    let shader = game.join("shaderpacks/shader.zip");
    directory_pack(&directory, "目录资源", &[1; 123]);
    let packed = pack_metadata("压缩资源");
    let expanded_payload = vec![0; 80_000];
    archive(
        &zip,
        &[
            ("pack.mcmeta", &packed),
            ("assets/large.bin", &expanded_payload),
        ],
    );
    mod_jar(&jar, "有名字的模组", 90_000);
    archive(&shader, &[("shaders/test.vsh", b"void main(){}")]);
    let report = scan(&settings(&game));
    let directory_bytes = fs::metadata(directory.join("pack.mcmeta")).unwrap().len() + 123;
    assert_eq!(at(&report.items, &directory).size, Some(directory_bytes));
    for path in [&zip, &jar, &shader] {
        let item = at(&report.items, path);
        assert_eq!(item.size, Some(fs::metadata(path).unwrap().len()));
        assert!(item.size_complete && item.available);
    }
    assert_eq!(at(&report.items, &jar).name, "有名字的模组");
    assert!(
        at(&report.items, &zip).size.unwrap() < 80_000,
        "archive size must not count expanded payloads"
    );
    assert_eq!(
        at(&report.items, &game).size,
        Some(
            directory_bytes
                + [&zip, &jar, &shader]
                    .iter()
                    .map(|path| fs::metadata(path).unwrap().len())
                    .sum::<u64>()
        )
    );
}

#[test]
fn incremental_change_updates_one_leaf_and_recomputes_only_its_instance_total() {
    let fixture = tempfile::tempdir().unwrap();
    let game = fixture.path().join("Minecraft");
    let a = game.join("mods/a.jar");
    let b = game.join("mods/b.jar");
    mod_jar(&a, "原名称", 40);
    mod_jar(&b, "不受影响", 50);
    world(&game.join("saves/world"), "不受影响的世界", 87);
    let settings = settings(&game);
    let previous = scan(&settings);
    let index = Index::open(&fixture.path().join("cache.db")).unwrap();
    cache(&index, &previous);
    let unchanged = at(&previous.items, &b).clone();
    let original_world = at(&previous.items, &game.join("saves/world")).clone();
    mod_jar(&a, "明确的新名称", 100_000);
    let mut updated = Vec::new();
    let refresh = scanner::refresh_changed(
        &settings,
        &previous.items,
        std::slice::from_ref(&a),
        &AtomicBool::new(false),
        |event| {
            if let ScanEvent::Updated(item) = event {
                updated.push(item.id);
            }
        },
    );
    assert!(!refresh.scan.cancelled);
    assert_eq!(refresh.scan.items.len(), 2);
    let changed = at(&refresh.scan.items, &a);
    assert_eq!(changed.name, "明确的新名称");
    assert_eq!(changed.size, Some(fs::metadata(&a).unwrap().len()));
    assert_eq!(updated.len(), 2);
    assert!(updated.contains(&changed.id));
    assert!(updated.contains(&at(&refresh.scan.items, &game).id));
    assert!(refresh.invalidated.is_empty());
    let aggregate = at(&refresh.scan.items, &game);
    assert!(aggregate.size.is_some() && aggregate.size_complete);
    assert_eq!(
        aggregate.size,
        Some(
            scanner::directory_size(&game, &ItemKind::Instance, true, &AtomicBool::new(false))
                .unwrap()
                .0
        )
    );
    assert!(refresh.removed_ids.is_empty());
    apply_refresh(&index, &refresh, 2);
    let stored_b = index.get(&unchanged.id).unwrap().unwrap();
    assert_eq!(stored_b.name, unchanged.name);
    assert_eq!(stored_b.size, unchanged.size);
    assert_eq!(stored_b.modified_ms, unchanged.modified_ms);
    assert_eq!(
        index.get(&original_world.id).unwrap().unwrap().size,
        original_world.size
    );
    assert_eq!(
        index.get(&aggregate.id).unwrap().unwrap().size,
        aggregate.size
    );
}

#[test]
fn incremental_create_rename_and_delete_remove_only_confirmed_content_paths() {
    let fixture = tempfile::tempdir().unwrap();
    let game = fixture.path().join("Minecraft");
    let existing = game.join("mods/existing.jar");
    mod_jar(&existing, "保留", 11);
    let settings = settings(&game);
    let initial = scan(&settings);
    let index = Index::open(&fixture.path().join("cache.db")).unwrap();
    cache(&index, &initial);
    let created = game.join("mods/added.jar");
    mod_jar(&created, "新增", 19);
    let added = scanner::refresh_changed(
        &settings,
        &index.query(&Query::default()).unwrap(),
        std::slice::from_ref(&created),
        &AtomicBool::new(false),
        |_| {},
    );
    assert!(added.removed_ids.is_empty());
    assert_eq!(added.scan.items.len(), 2);
    let added_id = at(&added.scan.items, &created).id.clone();
    apply_refresh(&index, &added, 2);

    let renamed = game.join("mods/renamed.jar");
    fs::rename(&created, &renamed).unwrap();
    let rename = scanner::refresh_changed(
        &settings,
        &index.query(&Query::default()).unwrap(),
        &[created.clone(), renamed.clone()],
        &AtomicBool::new(false),
        |_| {},
    );
    assert_eq!(
        rename.removed_ids.as_slice(),
        std::slice::from_ref(&added_id)
    );
    let renamed_id = at(&rename.scan.items, &renamed).id.clone();
    apply_refresh(&index, &rename, 3);
    assert!(index.get(&added_id).unwrap().is_none());
    assert!(index.get(&renamed_id).unwrap().is_some());

    fs::remove_file(&renamed).unwrap();
    let deleted = scanner::refresh_changed(
        &settings,
        &index.query(&Query::default()).unwrap(),
        std::slice::from_ref(&renamed),
        &AtomicBool::new(false),
        |_| {},
    );
    assert_eq!(
        deleted.removed_ids.as_slice(),
        std::slice::from_ref(&renamed_id)
    );
    assert_eq!(deleted.scan.items.len(), 1);
    assert_eq!(deleted.scan.items[0].kind, ItemKind::Instance);
    assert!(deleted.scan.items[0].size.is_some());
    apply_refresh(&index, &deleted, 4);
    assert!(index.get(&renamed_id).unwrap().is_none());
    assert_eq!(index.stats().unwrap().mods, 1);
    assert_eq!(
        index
            .get(&at(&initial.items, &existing).id)
            .unwrap()
            .unwrap()
            .name,
        "保留"
    );
}

#[test]
fn cancelling_after_discovery_preserves_existing_cached_items_and_sizes() {
    let fixture = tempfile::tempdir().unwrap();
    let game = fixture.path().join("Minecraft");
    let removed = game.join("mods/removed.jar");
    let changed = game.join("mods/changed.jar");
    mod_jar(&removed, "保留历史缓存", 11);
    mod_jar(&changed, "旧名称", 21);
    let settings = settings(&game);
    let initial = scan(&settings);
    let mut index = Index::open(&fixture.path().join("cache.db")).unwrap();
    cache(&index, &initial);
    fs::remove_file(&removed).unwrap();
    mod_jar(&changed, "不应提前覆盖的名称", 101);
    let cancel = AtomicBool::new(false);
    let mut updates = 0;
    let refresh = scanner::refresh_changed(
        &settings,
        &initial.items,
        &[removed.clone(), changed.clone()],
        &cancel,
        |event| match event {
            ScanEvent::Discovered(items) => {
                index.discovered(&items, 2).unwrap();
                cancel.store(true, Ordering::SeqCst);
            }
            ScanEvent::Updated(_) => updates += 1,
            _ => {}
        },
    );
    assert!(refresh.scan.cancelled);
    assert_eq!(updates, 0);
    assert!(refresh.scan.items.is_empty());
    // 清理候选仅为建议，任务取消后必须跳过它们。
    assert!(
        refresh
            .removed_ids
            .contains(&at(&initial.items, &removed).id)
    );
    assert!(
        index
            .get(&at(&initial.items, &removed).id)
            .unwrap()
            .is_some()
    );
    let old = at(&initial.items, &changed);
    let cached = index.get(&old.id).unwrap().unwrap();
    assert_eq!(cached.name, old.name);
    assert_eq!(cached.size, old.size);
    let old_instance = at(&initial.items, &game);
    assert_eq!(
        index.get(&old_instance.id).unwrap().unwrap().size,
        old_instance.size
    );
    assert_eq!(index.stats().unwrap().mods, 2);
    let retry = scanner::refresh_changed(
        &settings,
        &index.query(&Query::default()).unwrap(),
        &[removed, changed.clone()],
        &AtomicBool::new(false),
        |_| {},
    );
    apply_refresh(&index, &retry, 3);
    assert_eq!(index.stats().unwrap().mods, 1);
    assert_eq!(
        index.get(&old.id).unwrap().unwrap().name,
        "不应提前覆盖的名称"
    );
}

#[cfg(windows)]
#[test]
fn inaccessible_subtree_protects_cache_without_marking_healthy_items_offline() {
    use std::{path::PathBuf, process::Command};

    struct Junction(PathBuf);
    impl Drop for Junction {
        fn drop(&mut self) {
            let _ = fs::remove_dir(&self.0);
        }
    }
    fn junction(path: &Path, target: &Path) -> Junction {
        let output = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference='Stop'; New-Item -ItemType Junction -Path $env:ME_STAGE2_LINK -Target $env:ME_STAGE2_TARGET | Out-Null"])
            .env("ME_STAGE2_LINK", path)
            .env("ME_STAGE2_TARGET", target)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Junction(path.into())
    }

    let fixture = tempfile::tempdir().unwrap();
    let game = fixture.path().join("Minecraft");
    let other = fixture.path().join("外部目标");
    let offline = fixture.path().join("链接根目录");
    world(&game.join("saves/world"), "正常可读", 13);
    mod_jar(&game.join("mods/old.jar"), "已缓存模组", 17);
    mod_jar(&other.join("mods/outside.jar"), "不可跟随", 19);
    let previous = scan(&settings(&game));
    let index = Index::open(&fixture.path().join("cache.db")).unwrap();
    cache(&index, &previous);
    fs::remove_file(game.join("mods/old.jar")).unwrap();
    fs::remove_dir(game.join("mods")).unwrap();
    let _subtree_link = junction(&game.join("mods"), &other.join("mods"));
    let _source_link = junction(&offline, &other);
    let mut configured = settings(&game);
    configured.sources.push(Source {
        path: offline.clone(),
        kind: SourceKind::Root,
        associated_root: None,
    });
    let report = scan(&configured);
    let game_key = path_key(&game);
    let offline_key = path_key(&offline);
    assert!(report.failed_sources.contains(&game_key));
    assert!(!report.offline_sources.contains(&game_key));
    assert!(report.offline_sources.contains(&offline_key));
    assert!(!report.completed_sources.contains(&game_key));
    assert!(at(&report.items, &game.join("saves/world")).available);
    assert!(!report.items.iter().any(|item| item.name == "不可跟随"));
    for source in &report.completed_sources {
        index.finish_source(source, 2).unwrap();
    }
    assert!(
        index
            .get(&at(&previous.items, &game.join("mods/old.jar")).id)
            .unwrap()
            .is_some()
    );
    let refresh = scanner::refresh_changed(
        &configured,
        &previous.items,
        &[game.join("mods/old.jar")],
        &AtomicBool::new(false),
        |_| {},
    );
    assert!(refresh.removed_ids.is_empty());
    assert!(refresh.scan.offline_sources.contains(&offline_key));
}

#[test]
fn world_and_directory_pack_preview_timestamps_change_when_only_icons_change() {
    use image::ImageEncoder;

    fn png(value: u8) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(
                &[value; 16 * 16 * 4],
                16,
                16,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        bytes
    }
    fn set_mtime(path: &Path, seconds: u64) {
        let file = fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_times(
            fs::FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(seconds)),
        )
        .unwrap();
    }

    let fixture = tempfile::tempdir().unwrap();
    let game = fixture.path().join("Minecraft");
    let world_path = game.join("saves/world");
    let pack_path = game.join("resourcepacks/directory");
    world(&world_path, "世界", 10);
    directory_pack(&pack_path, "资源", &[1; 9]);
    let world_icon = world_path.join("icon.png");
    let pack_icon = pack_path.join("pack.png");
    for icon in [&world_icon, &pack_icon] {
        write(icon, png(16));
        set_mtime(icon, 1_700_000_001);
    }
    let settings = settings(&game);
    let before = scan(&settings);
    for path in [&world_path, &pack_path] {
        let item = at(&before.items, path);
        assert!(item.icon.is_some());
        assert_eq!(item.preview_modified_ms, Some(1_700_000_001_000));
    }
    for icon in [&world_icon, &pack_icon] {
        write(icon, png(128));
        set_mtime(icon, 1_700_000_010);
    }
    let refreshed = scanner::refresh_changed(
        &settings,
        &before.items,
        &[world_icon.clone(), pack_icon.clone()],
        &AtomicBool::new(false),
        |_| {},
    );
    assert_eq!(refreshed.scan.items.len(), 3);
    for (path, icon) in [(&world_path, &world_icon), (&pack_path, &pack_icon)] {
        let item = at(&refreshed.scan.items, path);
        assert_eq!(item.preview_modified_ms, modified_ms(icon));
        assert_eq!(item.preview_modified_ms, Some(1_700_000_010_000));
        assert_ne!(
            item.preview_modified_ms,
            at(&before.items, path).preview_modified_ms
        );
    }
    let mut old_json = serde_json::to_value(at(&before.items, &world_path)).unwrap();
    old_json
        .as_object_mut()
        .unwrap()
        .remove("preview_modified_ms");
    let migrated: Item = serde_json::from_value(old_json).unwrap();
    assert!(migrated.preview_modified_ms.is_none());
}
