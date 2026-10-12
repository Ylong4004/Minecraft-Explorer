use minecraft_explorer::{
    content,
    models::*,
    platform::{checked_path, path_key},
    scanner,
};
use std::{fs, io::Write, path::Path, sync::atomic::AtomicBool};
fn write(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}
fn zip(path: &Path, entries: &[(&str, &[u8])]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut archive = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for (name, bytes) in entries {
        archive
            .start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(bytes).unwrap();
    }
    archive.finish().unwrap();
}
#[test]
fn only_root_metadata_and_data_directory_are_data_packs() {
    let fixture = tempfile::tempdir().unwrap();
    let game = fixture.path().join("Minecraft");
    let folder = game.join("saves/world/datapacks");
    for name in [
        ".git",
        "tools",
        "metadata-only",
        "data-only",
        "data-file",
        "valid",
    ] {
        fs::create_dir_all(folder.join(name)).unwrap();
    }
    write(&folder.join("metadata-only/pack.mcmeta"), b"{}");
    write(&folder.join("data-file/pack.mcmeta"), b"{}");
    write(&folder.join("data-file/data"), b"not a directory");
    write(&folder.join("data-only/data/test/file.json"), b"{}");
    write(
        &folder.join("valid/pack.mcmeta"),
        b"invalid metadata still has required structure",
    );
    fs::create_dir(folder.join("valid/data")).unwrap();
    zip(
        &folder.join("valid.zip"),
        &[
            ("pack.mcmeta", b"{}"),
            ("data/test/functions/a.mcfunction", b"say hi"),
        ],
    );
    zip(
        &folder.join("missing-data.zip"),
        &[("pack.mcmeta", b"{}"), ("assets/a", b"x")],
    );
    zip(&folder.join("missing-metadata.zip"), &[("data/a", b"x")]);
    zip(
        &folder.join("nested.zip"),
        &[("wrapper/pack.mcmeta", b"{}"), ("wrapper/data/a", b"x")],
    );
    write(&folder.join("broken.zip"), b"broken zip");
    assert!(content::is_data_pack(&folder.join("valid")).unwrap());
    assert!(content::is_data_pack(&folder.join("valid.zip")).unwrap());
    let settings = Settings {
        sources: vec![Source {
            path: checked_path(&game).unwrap(),
            kind: SourceKind::Root,
            associated_root: None,
        }],
        ..Default::default()
    };
    let report = scanner::scan(&settings, &AtomicBool::new(false), |_| {});
    let mut names = report
        .items
        .iter()
        .filter(|i| i.kind == ItemKind::DataPack)
        .map(|i| i.folder.as_str())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(names, ["valid", "valid.zip"]);
}
#[test]
fn explicit_empty_data_directory_in_zip_is_accepted() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("empty.zip");
    let mut archive = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    archive
        .start_file("pack.mcmeta", zip::write::SimpleFileOptions::default())
        .unwrap();
    archive.write_all(b"{}").unwrap();
    archive
        .add_directory("data/", zip::write::SimpleFileOptions::default())
        .unwrap();
    archive.finish().unwrap();
    assert!(content::is_data_pack(&path).unwrap());
}
#[test]
fn incremental_refresh_removes_pack_when_required_structure_disappears() {
    let fixture = tempfile::tempdir().unwrap();
    let game = fixture.path().join("Minecraft");
    let pack = game.join("saves/world/datapacks/a");
    write(&pack.join("pack.mcmeta"), b"{}");
    fs::create_dir(pack.join("data")).unwrap();
    let settings = Settings {
        sources: vec![Source {
            path: checked_path(&game).unwrap(),
            kind: SourceKind::Root,
            associated_root: None,
        }],
        ..Default::default()
    };
    let scan = scanner::scan(&settings, &AtomicBool::new(false), |_| {});
    let id = scan
        .items
        .iter()
        .find(|i| i.kind == ItemKind::DataPack)
        .unwrap()
        .id
        .clone();
    fs::remove_dir(pack.join("data")).unwrap();
    let refresh = scanner::refresh_changed(
        &settings,
        &scan.items,
        &[pack.join("data")],
        &AtomicBool::new(false),
        |_| {},
    );
    assert!(refresh.removed_ids.contains(&id));
    assert!(pack.exists());
    write(&pack.join("data/test/f.json"), b"{}");
    let refresh = scanner::refresh_changed(
        &settings,
        &scan.items,
        &[pack.join("data")],
        &AtomicBool::new(false),
        |_| {},
    );
    assert!(
        refresh
            .scan
            .items
            .iter()
            .any(|i| i.kind == ItemKind::DataPack && path_key(&i.path) == path_key(&pack))
    );
}

#[test]
fn old_false_positive_cache_is_pruned_before_size_scan_without_losing_annotations() {
    use minecraft_explorer::{discovery::base_item, index::Index};
    let fixture = tempfile::tempdir().unwrap();
    let game = fixture.path().join("Minecraft");
    let path = game.join("saves/world/datapacks/.git");
    write(&path.join("config"), b"not a pack");
    let old = base_item(
        ItemKind::DataPack,
        checked_path(&path).unwrap(),
        game.clone(),
        path_key(&game),
    );
    let index = Index::open(&fixture.path().join("index.db")).unwrap();
    index.upsert(&old, 1).unwrap();
    index.annotate(&old.id, true, &["旧标签".into()]).unwrap();
    let invalid = scanner::invalid_data_pack_cache(std::slice::from_ref(&old), &[]);
    assert_eq!(invalid.as_slice(), std::slice::from_ref(&old.id));
    index.remove_items(&invalid).unwrap();
    assert!(index.get(&old.id).unwrap().is_none());
    assert!(path.join("config").exists());
    write(&path.join("pack.mcmeta"), b"{}");
    write(&path.join("data/test/a.json"), b"{}");
    index.upsert(&old, 2).unwrap();
    let restored = index.get(&old.id).unwrap().unwrap();
    assert!(restored.favorite);
    assert_eq!(restored.tags, ["旧标签"]);
    assert!(scanner::invalid_data_pack_cache(&[old], &[]).is_empty());
}
