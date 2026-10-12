use minecraft_explorer::{
    discovery::base_item,
    index::Index,
    launcher::{self, Capability, HmclAdapter, LauncherAdapter, Pcl2Adapter},
    minecraft::world_metadata,
    models::*,
    platform::*,
    scanner::{self, ScanEvent},
};
use serde::Serialize;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};

fn root(path: &Path) -> Source {
    Source {
        path: path.into(),
        kind: SourceKind::Root,
        associated_root: None,
    }
}
fn custom(path: &Path, owner: Option<PathBuf>) -> Source {
    Source {
        path: path.into(),
        kind: SourceKind::Custom,
        associated_root: owner,
    }
}
fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}
#[derive(Serialize)]
struct Nbt {
    #[serde(rename = "Data")]
    data: NbtData,
}
#[derive(Serialize)]
struct NbtData {
    #[serde(rename = "LevelName")]
    name: String,
    #[serde(rename = "LastPlayed")]
    last_played: i64,
    #[serde(rename = "Version")]
    version: NbtVersion,
}
#[derive(Serialize)]
struct NbtVersion {
    #[serde(rename = "Name")]
    name: String,
}
fn world(path: &Path, name: &str, time: i64, region_bytes: usize) {
    let bytes = fastnbt::to_bytes(&Nbt {
        data: NbtData {
            name: name.into(),
            last_played: time,
            version: NbtVersion {
                name: "1.21.11".into(),
            },
        },
    })
    .unwrap();
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&bytes).unwrap();
    write(&path.join("level.dat"), gzip.finish().unwrap());
    write(&path.join("region/r.0.0.mca"), vec![0u8; region_bytes]);
}
fn version(path: &Path, id: &str, inherits: Option<&str>) {
    let folder = path.file_name().unwrap().to_string_lossy();
    write(&path.join(format!("{folder}.json")),serde_json::to_vec(&serde_json::json!({"id":id,"inheritsFrom":inherits,"libraries":[{"name":"net.fabricmc:fabric-loader:0.19.3"}]})).unwrap());
    write(&path.join(format!("{folder}.jar")), [1, 2, 3]);
}
fn scan(sources: Vec<Source>) -> scanner::ScanReport {
    scanner::scan(
        &Settings {
            sources,
            ..Default::default()
        },
        &AtomicBool::new(false),
        |_| {},
    )
}
fn worlds(report: &scanner::ScanReport) -> Vec<&Item> {
    report
        .items
        .iter()
        .filter(|i| i.kind == ItemKind::World)
        .collect()
}

#[test]
fn isolated_same_names_and_shared_world_once() {
    let fixture = tempfile::tempdir().unwrap();
    let mc = checked_path(fixture.path()).unwrap().join("我的 Minecraft");
    let a = mc.join("versions/任意名称 Fabric");
    let b = mc.join("versions/另一个隔离实例");
    version(&a, "id-fabric", Some("1.21.11"));
    version(&b, "id-another", Some("1.21.11"));
    world(&a.join("saves/甲"), "冒险地图", 1_700_000_000_123, 13);
    world(&b.join("saves/乙"), "冒险地图", 1_700_000_010_123, 31);
    world(&mc.join("saves/共享"), "共享世界", 1_700_000_000_123, 7);
    let report = scan(vec![root(&mc), custom(&a, Some(mc.clone()))]);
    assert!(!report.cancelled);
    assert_eq!(worlds(&report).len(), 3);
    let duplicate_names: Vec<_> = worlds(&report)
        .into_iter()
        .filter(|w| w.name == "冒险地图")
        .collect();
    assert_eq!(duplicate_names.len(), 2);
    assert_ne!(duplicate_names[0].id, duplicate_names[1].id);
    assert!(
        duplicate_names
            .iter()
            .all(|w| w.game_dir.as_ref() != Some(&mc))
    );
    let shared = worlds(&report)
        .into_iter()
        .find(|w| w.name == "共享世界")
        .unwrap();
    assert_eq!(shared.version_id, None);
    assert!(shared.ownership.contains("共享"));
    assert_eq!(shared.last_played_ms, Some(1_700_000_000_123));
    assert_eq!(shared.game_version.as_deref(), Some("1.21.11"));
    assert_eq!(
        report
            .items
            .iter()
            .filter(|i| i.kind == ItemKind::Version)
            .count(),
        2
    );
}

#[test]
fn corrupted_missing_old_and_uncompressed_nbt() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let good = root.join("saves/正常");
    world(&good, "正常名称", 1_700_000_000_123, 10);
    fs::rename(good.join("level.dat"), good.join("level.dat_old")).unwrap();
    write(&good.join("level.dat"), b"corrupt");
    write(&root.join("saves/缺失/region/a.mca"), b"123456");
    write(&root.join("saves/损坏/level.dat"), b"bad nbt");
    let report = scan(vec![custom(root, None)]);
    assert_eq!(worlds(&report).len(), 3);
    let good = worlds(&report)
        .into_iter()
        .find(|i| i.name == "正常名称")
        .unwrap();
    assert!(good.metadata_status.contains("level.dat_old"));
    assert_eq!(fs::read(good.path.join("level.dat")).unwrap(), b"corrupt");
    let missing = worlds(&report)
        .into_iter()
        .find(|i| i.name == "缺失")
        .unwrap();
    assert_eq!(missing.size, Some(6));
    assert!(missing.metadata_status.contains("替代信息"));
    let raw = fastnbt::to_bytes(&Nbt {
        data: NbtData {
            name: "未压缩".into(),
            last_played: 1_700_000_000_123,
            version: NbtVersion {
                name: "raw-version".into(),
            },
        },
    })
    .unwrap();
    write(&root.join("raw/level.dat"), raw);
    assert_eq!(
        world_metadata(&root.join("raw")).name.as_deref(),
        Some("未压缩")
    );
}

#[test]
fn official_gamedir_and_explicit_custom_owner() {
    let fixture = tempfile::tempdir().unwrap();
    let mc = fixture.path().join(".minecraft");
    let external = checked_path(fixture.path()).unwrap().join("外部 实例");
    world(&external.join("saves/地图"), "外部", 1_700_000_000_123, 4);
    write(&mc.join("launcher_profiles.json"),serde_json::to_vec(&serde_json::json!({"profiles":{"profile":{"name":"自定义","gameDir":external}},"authenticationDatabase":{"ignore":"never indexed"}})).unwrap());
    let report = scan(vec![root(&mc), custom(&external, Some(mc.clone()))]);
    assert_eq!(worlds(&report).len(), 1);
    assert!(worlds(&report)[0].ownership.contains("官方"));
    assert_eq!(worlds(&report)[0].root, checked_path(&mc).unwrap());
}

#[test]
fn two_roots_preserve_same_folder_names() {
    let fixture = tempfile::tempdir().unwrap();
    let a = fixture.path().join("a");
    let b = fixture.path().join("b");
    world(&a.join("saves/同名"), "同名", 1_700_000_000_123, 8);
    world(&b.join("saves/同名"), "同名", 1_700_000_000_123, 8);
    let report = scan(vec![root(&a), root(&b)]);
    assert_eq!(worlds(&report).len(), 2);
    assert_ne!(worlds(&report)[0].id, worlds(&report)[1].id);
}

#[test]
fn version_core_and_game_data_do_not_overlap() {
    let fixture = tempfile::tempdir().unwrap();
    let mc = fixture.path();
    let v = mc.join("versions/a");
    version(&v, "custom-id", Some("base"));
    world(&v.join("saves/world"), "world", 1_700_000_000_123, 100);
    let report = scan(vec![root(mc)]);
    let definition = report
        .items
        .iter()
        .find(|i| i.kind == ItemKind::Version)
        .unwrap();
    let instance = report
        .items
        .iter()
        .find(|i| i.kind == ItemKind::Instance && i.path == definition.path)
        .unwrap();
    let complete = scanner::directory_size(
        &definition.path,
        &ItemKind::World,
        false,
        &AtomicBool::new(false),
    )
    .unwrap()
    .0;
    assert_eq!(definition.size.unwrap() + instance.size.unwrap(), complete);
    assert_eq!(definition.version_id.as_deref(), Some("custom-id"));
    assert_eq!(definition.loader.as_deref(), Some("Fabric"));
    let root_instance = report
        .items
        .iter()
        .find(|i| i.kind == ItemKind::Instance && i.path == checked_path(mc).unwrap())
        .unwrap();
    assert_eq!(root_instance.size, Some(0));
}

#[test]
fn unknown_version_no_directory_name_guess_and_empty_saves() {
    let fixture = tempfile::tempdir().unwrap();
    fs::create_dir_all(fixture.path().join("versions/1.99-fake")).unwrap();
    fs::create_dir_all(fixture.path().join("saves/空世界")).unwrap();
    let report = scan(vec![root(fixture.path())]);
    let version = report
        .items
        .iter()
        .find(|i| i.kind == ItemKind::Version)
        .unwrap();
    assert!(version.version_id.is_none());
    assert!(version.game_dir.is_none());
    assert_eq!(worlds(&report).len(), 1);
    assert_eq!(worlds(&report)[0].size, Some(0));
}

#[test]
fn missing_source_does_not_stop_other_roots() {
    let fixture = tempfile::tempdir().unwrap();
    world(
        &fixture.path().join("saves/world"),
        "ok",
        1_700_000_000_123,
        5,
    );
    let report = scan(vec![
        root(&fixture.path().join("missing")),
        root(fixture.path()),
    ]);
    assert_eq!(worlds(&report).len(), 1);
    assert_eq!(report.failed_sources.len(), 1);
    assert!(!report.warnings.is_empty());
}

#[test]
fn cancellation_mid_scan_and_retry() {
    let fixture = tempfile::tempdir().unwrap();
    for n in 0..12 {
        world(
            &fixture.path().join(format!("saves/{n}")),
            "world",
            1_700_000_000_123,
            10,
        );
    }
    let settings = Settings {
        sources: vec![root(fixture.path())],
        ..Default::default()
    };
    let cancel = AtomicBool::new(false);
    let report = scanner::scan(&settings, &cancel, |e| {
        if matches!(e, ScanEvent::Updated(_)) {
            cancel.store(true, Ordering::SeqCst);
        }
    });
    assert!(report.cancelled);
    assert_eq!(report.items.len(), 1);
    cancel.store(false, Ordering::SeqCst);
    assert_eq!(worlds(&scanner::scan(&settings, &cancel, |_| {})).len(), 12);
}

#[test]
fn sqlite_search_sort_literal_wildcards_incremental_and_offline_cache() {
    let fixture = tempfile::tempdir().unwrap();
    world(
        &fixture.path().join("saves/alpha_100%"),
        "冒险地图",
        1_700_000_000_123,
        10,
    );
    world(
        &fixture.path().join("saves/beta"),
        "冒险地图",
        1_700_000_000_999,
        500,
    );
    let report = scan(vec![root(fixture.path())]);
    let mut db = Index::open(&fixture.path().join("index.db")).unwrap();
    for item in &report.items {
        db.upsert(item, 1).unwrap();
    }
    let mut query = Query {
        text: "冒险".into(),
        kind: Some(ItemKind::World),
        sort: Sort::SizeDesc,
        ..Default::default()
    };
    let results = db.query(&query).unwrap();
    assert_eq!(results.len(), 2);
    assert!(results[0].size > results[1].size);
    query.sort = Sort::SizeAsc;
    assert_eq!(db.query(&query).unwrap()[0].folder, "alpha_100%");
    query.sort = Sort::TimeDesc;
    assert_eq!(db.query(&query).unwrap()[0].folder, "beta");
    query.text = "_100%".into();
    assert_eq!(db.query(&query).unwrap().len(), 1);
    query.text = "' OR 1=1--".into();
    assert_eq!(db.query(&query).unwrap().len(), 0);
    let mut pending = results[0].clone();
    pending.size = None;
    db.discovered(&[pending], 2).unwrap();
    assert_eq!(
        db.get(&results[0].id).unwrap().unwrap().size,
        results[0].size
    );
    db.finish_source(&results[0].source, 2).unwrap();
    assert_eq!(db.stats().unwrap().worlds, 1);
    db.mark_unavailable(&results[0].source).unwrap();
    assert!(!db.get(&results[0].id).unwrap().unwrap().available);
    drop(db);
    let db = Index::open(&fixture.path().join("index.db")).unwrap();
    assert_eq!(db.stats().unwrap().worlds, 1);
}

#[test]
fn settings_mappings_persist_without_authentication() {
    let fixture = tempfile::tempdir().unwrap();
    let db = Index::open(&fixture.path().join("db")).unwrap();
    let settings = Settings {
        sources: vec![root(fixture.path())],
        default_launcher: Some("pcl".into()),
        ..Default::default()
    };
    db.save_settings(&settings).unwrap();
    assert_eq!(db.load_settings().unwrap().sources.len(), 1);
    assert_eq!(
        db.load_settings().unwrap().default_launcher.as_deref(),
        Some("pcl")
    );
}

#[test]
fn manual_mapping_can_link_unisolated_definition_to_shared_directory() {
    let fixture = tempfile::tempdir().unwrap();
    let mc = checked_path(fixture.path()).unwrap();
    let v = mc.join("versions/vanilla");
    version(&v, "vanilla-id", None);
    world(&mc.join("saves/world"), "shared", 1_700_000_000_123, 1);
    let report = scan(vec![root(&mc)]);
    let definition = report
        .items
        .iter()
        .find(|i| i.kind == ItemKind::Version)
        .unwrap();
    assert!(definition.game_dir.is_none());
    let pcl = installation(&mc, LauncherKind::Pcl2, "pcl.exe");
    let mapping = launcher::create_mapping(definition, &pcl, definition, &mc).unwrap();
    assert_eq!(mapping.version_id, "vanilla-id");
    assert_eq!(mapping.game_dir, mc);
    let shared = worlds(&report)[0];
    assert!(launcher::create_mapping(shared, &pcl, definition, &v).is_err());
    assert!(launcher::create_mapping(shared, &pcl, definition, &mc).is_ok());
}

#[test]
fn custom_directory_explicit_owner_supports_mapping() {
    let fixture = tempfile::tempdir().unwrap();
    let mc = fixture.path().join("root");
    let v = mc.join("versions/definition");
    version(&v, "custom-id", None);
    let game = fixture.path().join("custom");
    world(&game.join("saves/world"), "external", 1_700_000_000_123, 1);
    let report = scan(vec![root(&mc), custom(&game, Some(mc.clone()))]);
    let shared = worlds(&report)[0];
    let definition = report
        .items
        .iter()
        .find(|i| i.kind == ItemKind::Version)
        .unwrap();
    let pcl = installation(&mc, LauncherKind::Pcl2, "pcl.exe");
    let mapping = launcher::create_mapping(shared, &pcl, definition, &game).unwrap();
    assert_eq!(mapping.root, checked_path(&mc).unwrap());
}

#[test]
fn long_unicode_path_and_metadata_bounds() {
    let fixture = tempfile::tempdir().unwrap();
    let mut deep = checked_path(fixture.path()).unwrap();
    for _ in 0..7 {
        deep = deep.join("很长的游戏实例目录-含空格 abcdefghijklmnopqrstuvwxyz");
    }
    world(
        &deep.join("saves/勇者传说"),
        "勇者传说",
        1_700_000_000_123,
        1,
    );
    let report = scan(vec![custom(&deep, None)]);
    assert_eq!(worlds(&report).len(), 1);
    assert!(display_path(&worlds(&report)[0].path).len() > 260);
    assert!(read_bounded(&worlds(&report)[0].path.join("level.dat"), 1).is_err());
    assert!(checked_path(Path::new("relative/path")).is_err());
}

#[test]
fn nbt_decompression_limit_does_not_crash() {
    let fixture = tempfile::tempdir().unwrap();
    let mut zip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    zip.write_all(&vec![0u8; 17 * 1024 * 1024]).unwrap();
    write(&fixture.path().join("level.dat"), zip.finish().unwrap());
    let meta = world_metadata(fixture.path());
    assert!(meta.name.is_none());
    assert!(meta.status.contains("超过 16 MiB"));
}

#[cfg(windows)]
#[test]
fn junction_cycle_is_skipped_and_explicit_alias_rejected() {
    let fixture = tempfile::tempdir().unwrap();
    let target = fixture.path().join("game");
    world(&target.join("saves/world"), "world", 1_700_000_000_123, 7);
    let link = target.join("saves/world/cycle");
    let status=std::process::Command::new("powershell.exe").args(["-NoProfile","-NonInteractive","-Command","$ErrorActionPreference='Stop'; New-Item -ItemType Junction -Path $env:ME_TEST_LINK -Target $env:ME_TEST_TARGET | Out-Null"])
        .env("ME_TEST_LINK",&link).env("ME_TEST_TARGET",&target).status().unwrap();
    assert!(status.success(), "junction fixture creation failed");
    let report = scan(vec![root(&target), custom(&link, None)]);
    assert_eq!(worlds(&report).len(), 1);
    assert!(!worlds(&report)[0].size_complete);
    assert!(checked_path(&link).is_err());
    assert!(!report.warnings.is_empty());
}

fn installation(fixture: &Path, kind: LauncherKind, file: &str) -> LauncherInstallation {
    let executable = fixture.join(file);
    write(&executable, b"fixture");
    LauncherInstallation {
        id: "test".into(),
        kind,
        executable,
        working_dir: fixture.into(),
        root: fixture.into(),
        java: None,
        checked_ms: 0,
    }
}

#[test]
fn pcl_exe_and_hmcl_jar_have_distinct_plans_and_open_only() {
    let fixture = tempfile::tempdir().unwrap();
    let pcl = installation(fixture.path(), LauncherKind::Pcl2, "PCL with 空格 &.exe");
    let plan = Pcl2Adapter.open_plan(&pcl).unwrap();
    assert_eq!(plan.program, pcl.executable);
    assert!(plan.args.is_empty());
    assert_eq!(Pcl2Adapter.capabilities(), &[Capability::OpenLauncher]);
    let mut hmcl = installation(fixture.path(), LauncherKind::Hmcl, "HMCL 中文 &.jar");
    assert!(HmclAdapter.open_plan(&hmcl).is_err());
    let java = fixture.path().join("Java 空格/javaw.exe");
    write(&java, b"java fixture");
    hmcl.java = Some(java.clone());
    let plan = HmclAdapter.open_plan(&hmcl).unwrap();
    assert_eq!(plan.program, java);
    assert_eq!(
        plan.args,
        vec![
            std::ffi::OsString::from("-jar"),
            hmcl.executable.into_os_string()
        ]
    );
    assert_eq!(plan.working_dir, fixture.path());
    assert!(
        !HmclAdapter
            .capabilities()
            .contains(&Capability::LaunchInstance)
    );
}

#[test]
fn mapping_cross_root_and_missing_target_are_rejected() {
    let fixture = tempfile::tempdir().unwrap();
    let pcl = installation(fixture.path(), LauncherKind::Pcl2, "pcl.exe");
    let v = fixture.path().join("versions/fabric");
    fs::create_dir_all(&v).unwrap();
    let mut version = base_item(
        ItemKind::Version,
        v.clone(),
        fixture.path().into(),
        "source".into(),
    );
    version.version_id = Some("id".into());
    let mut mapping = LaunchMapping {
        script: None,
        subject_id: "world".into(),
        launcher_id: pcl.id.clone(),
        root: fixture.path().into(),
        game_dir: fixture.path().into(),
        version_id: "id".into(),
        version_path: v,
        source: "手动".into(),
    };
    assert!(launcher::validate_mapping(&mapping, &pcl, &version).is_ok());
    assert!(
        launcher::launch_instance(&pcl, &mapping, &version)
            .unwrap_err()
            .to_string()
            .contains("OpenOnly")
    );
    mapping.root = fixture.path().join("other");
    assert!(launcher::validate_mapping(&mapping, &pcl, &version).is_err());
    mapping.root = fixture.path().into();
    mapping.version_id = "other-id".into();
    assert!(launcher::validate_mapping(&mapping, &pcl, &version).is_err());
}

#[test]
fn stress_1000_worlds_queries_and_cancel() {
    let fixture = tempfile::tempdir().unwrap();
    let mc = fixture.path().join("minecraft");
    for n in 0..1000 {
        world(
            &mc.join(format!("saves/world-{n:04}")),
            &format!("冒险-{n:04}"),
            1_700_000_000_000 + n,
            n as usize,
        );
    }
    let start = Instant::now();
    let report = scan(vec![root(&mc)]);
    let scan_elapsed = start.elapsed();
    assert_eq!(worlds(&report).len(), 1000);
    let mut db = Index::open(&fixture.path().join("stress.db")).unwrap();
    let start = Instant::now();
    db.discovered(&report.items, 1).unwrap();
    let index_elapsed = start.elapsed();
    let start = Instant::now();
    for _ in 0..20 {
        assert_eq!(
            db.query(&Query {
                text: "冒险-09".into(),
                kind: Some(ItemKind::World),
                sort: Sort::SizeDesc,
                ..Default::default()
            })
            .unwrap()
            .len(),
            100
        );
    }
    eprintln!(
        "1000 worlds: scan={scan_elapsed:?}, index={index_elapsed:?}, 20 search+sort={:?}",
        start.elapsed()
    );
}

#[cfg(windows)]
#[test]
fn locked_level_dat_keeps_world_visible() {
    use std::os::windows::fs::OpenOptionsExt;
    let fixture = tempfile::tempdir().unwrap();
    let dir = fixture.path().join("saves/被游戏占用");
    world(&dir, "原始名字", 1_700_000_000_123, 20);
    let lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(dir.join("level.dat"))
        .unwrap();
    let report = scan(vec![root(fixture.path())]);
    assert_eq!(worlds(&report).len(), 1);
    assert_eq!(worlds(&report)[0].name, "被游戏占用");
    assert!(worlds(&report)[0].metadata_status.contains("替代信息"));
    drop(lock);
    assert_eq!(world_metadata(&dir).name.as_deref(), Some("原始名字"));
}

#[cfg(windows)]
#[test]
fn denied_directory_does_not_hide_other_sources() {
    use std::process::{Command, Stdio};
    let fixture = tempfile::tempdir().unwrap();
    let denied = fixture.path().join("denied");
    let allowed = fixture.path().join("allowed");
    fs::create_dir_all(&denied).unwrap();
    world(
        &allowed.join("saves/world"),
        "allowed",
        1_700_000_000_123,
        1,
    );
    let status = Command::new("icacls.exe")
        .arg(&denied)
        .args(["/deny", "*S-1-1-0:(RD)"])
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let report = scan(vec![root(&denied), root(&allowed)]);
    // 断言或释放前始终恢复临时测试目录的访问控制。
    let restored = Command::new("icacls.exe")
        .arg(&denied)
        .args(["/remove:d", "*S-1-1-0"])
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(restored.success());
    assert_eq!(worlds(&report).len(), 1);
    assert_eq!(report.failed_sources.len(), 1);
}
