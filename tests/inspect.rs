use std::{fs, process::Command};

#[test]
fn inspect_normalizes_versions_and_rejects_game_cache() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("游戏 根目录");
    let versions = root.join("versions");
    fs::create_dir_all(root.join("saves/空存档")).unwrap();
    fs::create_dir_all(&versions).unwrap();
    let forbidden = root.join("不应创建的缓存");
    let output = Command::new(env!("CARGO_BIN_EXE_inspect"))
        .args([&versions, &forbidden])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!forbidden.exists());
    let cache = fixture.path().join("独立缓存");
    let output = Command::new(env!("CARGO_BIN_EXE_inspect"))
        .args([&versions, &cache])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(cache.join("report.json")).unwrap()).unwrap();
    assert_eq!(report["worlds"], 1);
    assert_eq!(report["versions"], 0);
    assert!(
        root.join("saves/空存档")
            .read_dir()
            .unwrap()
            .next()
            .is_none()
    );
}
