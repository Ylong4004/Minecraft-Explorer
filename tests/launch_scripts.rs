use minecraft_explorer::{
    discovery::base_item, index::Index, launcher, models::*, platform::checked_path,
};
use std::fs;

fn fixture(path: &std::path::Path) -> (LauncherInstallation, Item, LaunchMapping) {
    let root = path.join("Minecraft");
    let version_path = root.join("versions/chosen");
    fs::create_dir_all(version_path.join("saves")).unwrap();
    fs::write(version_path.join("chosen.json"), r#"{"id":"chosen"}"#).unwrap();
    let root = checked_path(&root).unwrap();
    let version_path = checked_path(&version_path).unwrap();
    let installation = LauncherInstallation {
        id: "pcl".into(),
        kind: LauncherKind::Pcl2,
        executable: path.join("launcher.exe"),
        working_dir: path.into(),
        root: root.clone(),
        java: None,
        checked_ms: 0,
    };
    let mut version = base_item(
        ItemKind::Version,
        version_path.clone(),
        root,
        "fixture".into(),
    );
    version.version_id = Some("chosen".into());
    version.game_dir = Some(version_path.clone());
    let mapping =
        launcher::create_mapping(&version, &installation, &version, &version_path).unwrap();
    (installation, version, mapping)
}

#[test]
fn binding_persists_and_older_mappings_have_no_script() {
    let directory = tempfile::tempdir().unwrap();
    let (_, _, mut mapping) = fixture(directory.path());
    let old = serde_json::to_value(&mapping).unwrap();
    let mut old = old.as_object().unwrap().clone();
    old.remove("script");
    assert!(
        serde_json::from_value::<LaunchMapping>(old.into())
            .unwrap()
            .script
            .is_none()
    );
    mapping.script = Some(LaunchScript {
        path: directory.path().join("chosen.cmd"),
        working_dir: directory.path().into(),
    });
    let db = Index::open(&directory.path().join("cache.db")).unwrap();
    let settings = Settings {
        mappings: vec![mapping],
        ..Default::default()
    };
    db.save_settings(&settings).unwrap();
    assert_eq!(
        db.load_settings().unwrap().mappings[0]
            .script
            .as_ref()
            .unwrap()
            .path,
        directory.path().join("chosen.cmd")
    );
}

fn pcl_script(mapping: &LaunchMapping) -> String {
    format!(
        "@echo off\r\n\"C:\\Fixture Java\\bin\\javaw.exe\" --version \"{}\" --gameDir \"{}\" --accessToken fixture-only\r\n",
        mapping.version_id,
        minecraft_explorer::platform::display_path(&mapping.game_dir)
    )
}

#[test]
fn auto_detection_checks_target_and_keeps_a_stable_local_copy() {
    let directory = tempfile::tempdir().unwrap();
    let (mut installation, _, mapping) = fixture(directory.path());
    installation.kind = LauncherKind::PclCe;
    fs::create_dir_all(directory.path().join("PCL")).unwrap();
    let original = directory.path().join("PCL/LatestLaunch.bat");
    let content = pcl_script(&mapping);
    fs::write(&original, &content).unwrap();
    let data = directory.path().join("app-data");
    fs::create_dir(&data).unwrap();
    let found = minecraft_explorer::launch_scripts::discover(&installation, &mapping, &data)
        .unwrap()
        .unwrap();
    assert_eq!(found.working_dir, checked_path(directory.path()).unwrap());
    assert!(found.path.starts_with(checked_path(&data).unwrap()));
    assert_eq!(fs::read(&found.path).unwrap(), content.as_bytes());
    let repeated = minecraft_explorer::launch_scripts::discover(&installation, &mapping, &data)
        .unwrap()
        .unwrap();
    assert_eq!(found.path, repeated.path);
    fs::write(
        &original,
        content.replace("--version \"chosen\"", "--version \"other-version\""),
    )
    .unwrap();
    assert!(
        minecraft_explorer::launch_scripts::discover(&installation, &mapping, &data)
            .unwrap()
            .is_none()
    );
    assert_eq!(fs::read(&found.path).unwrap(), content.as_bytes());
}

#[test]
fn auto_detection_rejects_other_directories_variables_comments_and_ambiguity() {
    let directory = tempfile::tempdir().unwrap();
    let (installation, _, mapping) = fixture(directory.path());
    fs::create_dir_all(directory.path().join("PCL")).unwrap();
    let original = directory.path().join("PCL/LatestLaunch.bat");
    let data = directory.path().join("app-data");
    fs::create_dir(&data).unwrap();
    let content = pcl_script(&mapping);
    let game = minecraft_explorer::platform::display_path(&mapping.game_dir);
    for invalid in [
        content.replace(
            &game,
            &minecraft_explorer::platform::display_path(&installation.root),
        ),
        content.replace(&game, "%GAME_DIRECTORY%"),
        content.replace("\"C:\\Fixture", "rem \"C:\\Fixture"),
        content.replace("--version", "--version other --version"),
        format!("{content}{content}"),
    ] {
        fs::write(&original, invalid).unwrap();
        assert!(
            minecraft_explorer::launch_scripts::discover(&installation, &mapping, &data)
                .unwrap()
                .is_none()
        );
        assert!(!data.join("launch-scripts").exists());
    }
}

#[test]
fn auto_detection_does_not_overwrite_edited_snapshots_or_scan_unrelated_launchers() {
    let directory = tempfile::tempdir().unwrap();
    let (mut installation, _, mapping) = fixture(directory.path());
    fs::create_dir_all(directory.path().join("PCL")).unwrap();
    let original = directory.path().join("PCL/LatestLaunch.bat");
    fs::write(&original, pcl_script(&mapping)).unwrap();
    let data = directory.path().join("app-data");
    fs::create_dir(&data).unwrap();
    let found = minecraft_explorer::launch_scripts::discover(&installation, &mapping, &data)
        .unwrap()
        .unwrap();
    fs::write(&found.path, "user content").unwrap();
    assert!(minecraft_explorer::launch_scripts::discover(&installation, &mapping, &data).is_err());
    assert_eq!(fs::read_to_string(&found.path).unwrap(), "user content");
    installation.kind = LauncherKind::Hmcl;
    assert!(
        minecraft_explorer::launch_scripts::discover(&installation, &mapping, &data)
            .unwrap()
            .is_none()
    );
}

#[test]
fn invalid_script_and_cross_root_binding_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let (installation, mut version, mut mapping) = fixture(directory.path());
    let path = directory.path().join("not-script.exe");
    fs::write(&path, "fixture").unwrap();
    mapping.script = Some(LaunchScript {
        path,
        working_dir: directory.path().into(),
    });
    assert!(launcher::spawn_script(&installation, &mapping, &version).is_err());
    let path = directory.path().join("chosen.cmd");
    fs::write(&path, "@exit /b 0").unwrap();
    mapping.script.as_mut().unwrap().path = path;
    version.root = directory.path().join("other-root");
    assert!(launcher::spawn_script(&installation, &mapping, &version).is_err());
}

#[cfg(windows)]
#[test]
fn real_selected_batch_runs_in_chosen_workdir_with_special_characters() {
    let directory = tempfile::tempdir().unwrap();
    let (installation, version, mut mapping) = fixture(directory.path());
    let folder = directory.path().join("导出的脚本 & (测试)");
    fs::create_dir_all(&folder).unwrap();
    let work = directory.path().join("工作目录 & (测试)");
    fs::create_dir_all(&work).unwrap();
    let script = folder.join("chosen & (literal).cmd");
    fs::write(
        &script,
        "@echo off\r\n>marker.txt echo chosen-version\r\nexit /b 0\r\n",
    )
    .unwrap();
    mapping.script = Some(LaunchScript {
        path: script,
        working_dir: work.clone(),
    });
    assert!(
        launcher::spawn_script(&installation, &mapping, &version)
            .unwrap()
            .wait()
            .unwrap()
            .success()
    );
    assert_eq!(
        fs::read_to_string(work.join("marker.txt")).unwrap().trim(),
        "chosen-version"
    );
    assert!(!folder.join("marker.txt").exists());
    assert!(!directory.path().join("marker.txt").exists());
}
