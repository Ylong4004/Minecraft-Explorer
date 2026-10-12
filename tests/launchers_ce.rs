use minecraft_explorer::{
    discovery::base_item,
    index::Index,
    launcher::{self, Capability, HmclAdapter, LauncherAdapter, Pcl2Adapter, PclCeAdapter},
    models::*,
    platform::checked_path,
};
use std::{fs, path::Path};

fn installation(root: &Path, id: &str) -> LauncherInstallation {
    let working_dir = root.join(format!("启动器 {id} & 空格"));
    fs::create_dir_all(&working_dir).unwrap();
    let executable = working_dir.join("PCL CE.exe");
    fs::write(&executable, b"test fixture").unwrap();
    LauncherInstallation {
        id: id.into(),
        kind: LauncherKind::PclCe,
        executable,
        working_dir,
        root: checked_path(root).unwrap(),
        java: None,
        checked_ms: 0,
    }
}

fn version(root: &Path) -> Item {
    let root = checked_path(root).unwrap();
    let path = root.join("versions/同名实例");
    fs::create_dir_all(path.join("saves/同名存档")).unwrap();
    let mut item = base_item(ItemKind::Version, path.clone(), root, "fixture".into());
    item.version_id = Some("same-version-id".into());
    item.game_dir = Some(path);
    item
}

#[test]
fn pcl_ce_is_a_distinct_exe_only_open_adapter() {
    let fixture = tempfile::tempdir().unwrap();
    let installation = installation(fixture.path(), "ce");
    assert_eq!(LauncherKind::PclCe.label(), "PCL CE");
    assert_eq!(
        launcher::adapter(&LauncherKind::PclCe).capabilities(),
        [Capability::OpenLauncher]
    );
    let plan = PclCeAdapter.open_plan(&installation).unwrap();
    assert_eq!(plan.program, installation.executable);
    assert_eq!(plan.working_dir, installation.working_dir);
    assert!(plan.args.is_empty());

    let jar = installation.working_dir.join("community.jar");
    fs::write(&jar, b"jar fixture").unwrap();
    let invalid = LauncherInstallation {
        executable: jar,
        ..installation.clone()
    };
    assert!(PclCeAdapter.open_plan(&invalid).is_err());
    assert!(Pcl2Adapter.open_plan(&installation).is_err());
    assert!(HmclAdapter.open_plan(&installation).is_err());
    let wrong_kind = LauncherInstallation {
        kind: LauncherKind::Pcl2,
        ..installation
    };
    assert!(PclCeAdapter.open_plan(&wrong_kind).is_err());
}

#[test]
fn pcl_ce_mapping_rejects_other_installations_roots_and_same_named_versions() {
    let fixture = tempfile::tempdir().unwrap();
    let root_a = fixture.path().join("根目录 A");
    let root_b = fixture.path().join("根目录 B");
    fs::create_dir_all(&root_a).unwrap();
    fs::create_dir_all(&root_b).unwrap();
    let ce_a = installation(&root_a, "ce-a");
    let ce_same_root = installation(&root_a, "ce-a-alternative");
    let ce_b = installation(&root_b, "ce-b");
    let version_a = version(&root_a);
    let version_b = version(&root_b);
    let game_a = version_a.game_dir.as_ref().unwrap();
    let mapping = launcher::create_mapping(&version_a, &ce_a, &version_a, game_a).unwrap();
    assert!(launcher::validate_mapping(&mapping, &ce_a, &version_a).is_ok());
    assert!(launcher::validate_mapping(&mapping, &ce_same_root, &version_a).is_err());
    assert!(launcher::validate_mapping(&mapping, &ce_b, &version_b).is_err());
    assert!(launcher::create_mapping(&version_a, &ce_b, &version_a, game_a).is_err());
    assert!(launcher::create_mapping(&version_a, &ce_a, &version_b, game_a).is_err());

    let fallback = launcher::create_mapping(&version_a, &ce_same_root, &version_a, game_a).unwrap();
    assert_eq!(fallback.launcher_id, "ce-a-alternative");
    assert_eq!(fallback.game_dir, mapping.game_dir);
    let gated = launcher::launch_instance(&ce_a, &mapping, &version_a).unwrap_err();
    assert!(gated.to_string().contains("OpenOnly"));
    assert!(
        !PclCeAdapter
            .capabilities()
            .contains(&Capability::LaunchInstance)
    );
    assert!(
        !PclCeAdapter
            .capabilities()
            .contains(&Capability::TrackGameProcess)
    );
}

#[test]
fn mixed_launcher_installations_and_individual_defaults_persist() {
    let fixture = tempfile::tempdir().unwrap();
    let ce = installation(fixture.path(), "ce-installation");
    let version = version(fixture.path());
    let mut pcl = ce.clone();
    pcl.id = "pcl-installation".into();
    pcl.kind = LauncherKind::Pcl2;
    let mut hmcl = ce.clone();
    hmcl.id = "hmcl-installation".into();
    hmcl.kind = LauncherKind::Hmcl;
    let mapping =
        launcher::create_mapping(&version, &ce, &version, version.game_dir.as_ref().unwrap())
            .unwrap();
    let settings = Settings {
        launchers: vec![pcl, hmcl, ce],
        mappings: vec![mapping],
        default_launcher: Some("pcl-installation".into()),
        instance_defaults: [(version.id.clone(), "ce-installation".into())]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    let database = fixture.path().join("test-index.db");
    let index = Index::open(&database).unwrap();
    index.save_settings(&settings).unwrap();
    drop(index);
    let loaded = Index::open(&database).unwrap().load_settings().unwrap();
    assert_eq!(loaded.launchers.len(), 3);
    assert_eq!(loaded.launchers[0].kind, LauncherKind::Pcl2);
    assert_eq!(loaded.launchers[1].kind, LauncherKind::Hmcl);
    assert_eq!(loaded.launchers[2].kind, LauncherKind::PclCe);
    assert_eq!(loaded.default_launcher.as_deref(), Some("pcl-installation"));
    assert_eq!(loaded.instance_defaults[&version.id], "ce-installation");
    assert_eq!(loaded.mappings[0].launcher_id, "ce-installation");
    for (kind, value) in [
        (LauncherKind::Pcl2, "\"Pcl2\""),
        (LauncherKind::Hmcl, "\"Hmcl\""),
        (LauncherKind::PclCe, "\"PclCe\""),
    ] {
        assert_eq!(serde_json::to_string(&kind).unwrap(), value);
        assert_eq!(serde_json::from_str::<LauncherKind>(value).unwrap(), kind);
    }
}

#[cfg(windows)]
#[test]
fn pcl_ce_process_opens_the_selected_installation_without_invented_arguments() {
    use std::process::Command;

    let fixture = tempfile::tempdir().unwrap();
    let root_a = fixture.path().join("Minecraft A & $literal;");
    let root_b = fixture.path().join("Minecraft B 中文 空格");
    fs::create_dir_all(&root_a).unwrap();
    fs::create_dir_all(&root_b).unwrap();
    let ce_a = installation(&root_a, "a");
    let ce_b = installation(&root_b, "b");
    let source = fixture.path().join("pcl_ce_fixture.rs");
    fs::write(
        &source,
        r#"
        fn main() {
            let arguments: Vec<_> = std::env::args().skip(1).collect();
            std::fs::write("arguments.txt", arguments.join("\n")).unwrap();
            std::fs::write("cwd.txt", std::env::current_dir().unwrap().to_string_lossy().as_bytes()).unwrap();
            std::fs::write("executable.txt", std::env::current_exe().unwrap().to_string_lossy().as_bytes()).unwrap();
        }
        "#,
    )
    .unwrap();
    assert!(
        Command::new("rustc")
            .arg(&source)
            .arg("-o")
            .arg(&ce_a.executable)
            .status()
            .unwrap()
            .success()
    );
    fs::copy(&ce_a.executable, &ce_b.executable).unwrap();
    let broken = installation(&root_a, "broken");
    let broken_plan = launcher::adapter(&broken.kind).open_plan(&broken).unwrap();
    assert!(launcher::spawn(&broken_plan).is_err());
    // 将失败返回调用方，改选其他安装仍由用户决定。
    assert!(!ce_a.working_dir.join("arguments.txt").exists());
    assert!(!ce_b.working_dir.join("arguments.txt").exists());
    for selected in [&ce_a, &ce_b] {
        let plan = launcher::adapter(&selected.kind)
            .open_plan(selected)
            .unwrap();
        assert!(launcher::spawn(&plan).unwrap().wait().unwrap().success());
        assert_eq!(
            fs::read_to_string(selected.working_dir.join("arguments.txt")).unwrap(),
            ""
        );
        assert_eq!(
            fs::read_to_string(selected.working_dir.join("cwd.txt")).unwrap(),
            selected.working_dir.to_string_lossy()
        );
        let reported_exe = fs::read_to_string(selected.working_dir.join("executable.txt")).unwrap();
        assert_eq!(
            checked_path(Path::new(&reported_exe)).unwrap(),
            checked_path(&selected.executable).unwrap()
        );
    }
}
