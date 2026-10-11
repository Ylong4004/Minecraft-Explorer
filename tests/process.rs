use minecraft_explorer::{
    launcher::{self, HmclAdapter, LauncherAdapter, Pcl2Adapter},
    models::*,
};
use std::{fs, process::Command};

#[test]
fn real_fixture_process_preserves_arguments_and_workdir() {
    let fixture = tempfile::tempdir().unwrap();
    let dir = fixture.path().join("启动器 & 有空格");
    fs::create_dir_all(&dir).unwrap();
    let source = dir.join("fixture.rs");
    fs::write(
        &source,
        r#"
        fn main() {
            let cwd = std::env::current_dir().unwrap();
            let args: Vec<_> = std::env::args().skip(1).collect();
            std::fs::write("cwd.txt", cwd.to_string_lossy().as_bytes()).unwrap();
            std::fs::write("args.txt", args.join("\n")).unwrap();
        }
    "#,
    )
    .unwrap();
    let exe = dir.join("假 PCL &.exe");
    assert!(
        Command::new("rustc")
            .arg(&source)
            .arg("-o")
            .arg(&exe)
            .status()
            .unwrap()
            .success()
    );
    let installation = LauncherInstallation {
        id: "fixture".into(),
        kind: LauncherKind::Pcl2,
        executable: exe.clone(),
        working_dir: dir.clone(),
        root: fixture.path().into(),
        java: None,
        checked_ms: 0,
    };
    let plan = Pcl2Adapter.open_plan(&installation).unwrap();
    assert!(launcher::spawn(&plan).unwrap().wait().unwrap().success());
    assert_eq!(fs::read_to_string(dir.join("args.txt")).unwrap(), "");
    assert_eq!(
        fs::read_to_string(dir.join("cwd.txt")).unwrap(),
        dir.to_string_lossy()
    );
    let java = dir.join("javaw.exe");
    fs::copy(&exe, &java).unwrap();
    let jar = dir.join("中文 目标; & ${literal}.jar");
    fs::write(&jar, b"fixture jar").unwrap();
    let installation = LauncherInstallation {
        kind: LauncherKind::Hmcl,
        executable: jar.clone(),
        java: Some(java),
        ..installation
    };
    let plan = HmclAdapter.open_plan(&installation).unwrap();
    assert!(launcher::spawn(&plan).unwrap().wait().unwrap().success());
    assert_eq!(
        fs::read_to_string(dir.join("args.txt")).unwrap(),
        format!("-jar\n{}", jar.display())
    );
}
