use minecraft_explorer::licenses;

#[test]
fn single_exe_contains_project_and_upstream_license_texts() {
    let bytes = licenses::texts().unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains(include_str!("../LICENSE")));
    assert!(text.contains("LicenseRef-Slint-Royalty-free-2.0"));
    assert!(text.contains("libsqlite3-sys"));
    assert!(text.contains("Unicode"));
    assert!(text.contains("embed-resource"));
}

#[test]
fn license_export_roundtrips_and_does_not_replace_existing_content() {
    let directory = tempfile::tempdir().unwrap();
    let path = licenses::export(directory.path()).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), licenses::texts().unwrap());
    assert_eq!(licenses::export(directory.path()).unwrap(), path);
    std::fs::write(&path, b"user-edited text").unwrap();
    assert!(licenses::export(directory.path()).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"user-edited text");
}
