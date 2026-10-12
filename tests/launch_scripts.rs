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
