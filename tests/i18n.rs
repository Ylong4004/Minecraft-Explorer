use minecraft_explorer::{
    i18n::{CATALOG, translate},
    index::Index,
    models::Settings,
};

#[test]
fn translation_keeps_dynamic_paths_and_names_unchanged() {
    let path = r"E:\中文目录 & $literal\计算中";
    assert_eq!(
        translate(&format!("无法打开启动器：{path}"), true),
        format!("Could not open launcher: {path}")
    );
    assert_eq!(
        translate("用户自己的中文地图名", true),
        "用户自己的中文地图名"
    );
    assert_eq!(translate("重命名", false), "重命名");
    assert_eq!(translate("重命名", true), "Rename");
    assert_eq!(
        translate(
            "操作失败：系统错误\n已取消 Windows 管理员授权，启动器未打开",
            true
        ),
        "Operation failed: 系统错误\nWindows administrator permission cancelled; launcher not opened"
    );
}

#[test]
fn translation_templates_have_matching_argument_counts() {
    let regex = regex::Regex::new(r"\{[^{}]*\}").unwrap();
    for (chinese, english) in CATALOG.iter() {
        assert_eq!(
            regex.find_iter(chinese).count(),
            regex.find_iter(english).count(),
            "{chinese}"
        );
    }
    let ui = include_str!("../ui/app.slint");
    let calls = regex::Regex::new(r#"I18n\.text\(("(?:[^"\\]|\\.)*")\)"#).unwrap();
    for capture in calls.captures_iter(ui) {
        let value: String = serde_json::from_str(&capture[1]).unwrap();
        assert!(CATALOG.contains_key(&value), "缺少界面翻译：{value}");
    }
}

#[test]
fn old_settings_default_to_chinese_dark_and_preferences_persist() {
    let settings: Settings = serde_json::from_str(
        r#"{"sources":[],"launchers":[],"mappings":[],"default_launcher":null}"#,
    )
    .unwrap();
    assert!(!settings.english && !settings.light_theme);
    let fixture = tempfile::tempdir().unwrap();
    let db = Index::open(&fixture.path().join("index.db")).unwrap();
    let settings = Settings {
        english: true,
        light_theme: true,
        ..settings
    };
    db.save_settings(&settings).unwrap();
    let loaded = db.load_settings().unwrap();
    assert!(loaded.english && loaded.light_theme);
}
