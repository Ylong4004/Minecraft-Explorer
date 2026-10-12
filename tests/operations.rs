use minecraft_explorer::{
    discovery::base_item,
    index::Index,
    models::*,
    operations::{self, Action, Conflict, Policy, VersionScope},
    platform::{checked_path, path_key},
    scanner,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
struct Fixture {
    _temp: tempfile::TempDir,
    a: PathBuf,
    b: PathBuf,
    data: PathBuf,
    backup: PathBuf,
    settings: Settings,
    policy: Policy,
}
fn write(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let a = temp.path().join("Minecraft 甲");
        let b = temp.path().join("Minecraft 乙");
        let data = temp.path().join("app-data");
        let backup = temp.path().join("备份");
        for root in [&a, &b] {
            for folder in ["saves", "mods", "resourcepacks", "shaderpacks", "versions"] {
                fs::create_dir_all(root.join(folder)).unwrap();
            }
        }
        fs::create_dir(&data).unwrap();
        fs::create_dir(&backup).unwrap();
        let settings = Settings {
            sources: vec![
                Source {
                    path: a.clone(),
                    kind: SourceKind::Root,
                    associated_root: None,
                },
                Source {
                    path: b.clone(),
                    kind: SourceKind::Root,
                    associated_root: None,
                },
            ],
            ..Default::default()
        };
        let games = [&a, &b]
            .iter()
            .map(|root| {
                let mut item = base_item(
                    ItemKind::Instance,
                    (*root).clone(),
                    (*root).clone(),
                    path_key(root),
                );
                item.game_dir = Some((*root).clone());
                item
            })
            .collect::<Vec<_>>();
        let policy = Policy::new(&settings, &games, &data).unwrap();
        Self {
            _temp: temp,
            a,
            b,
            data,
            backup,
            settings,
            policy,
        }
    }
    fn world(&self, name: &str) -> Item {
        let path = self.a.join("saves").join(name);
        write(&path.join("level.dat"), b"fixture world");
        write(&path.join("region/r.0.0.mca"), &vec![7; 10000]);
        self.item(&path, ItemKind::World)
    }
    fn item(&self, path: &Path, kind: ItemKind) -> Item {
        let path = checked_path(path).unwrap();
        let mut item = base_item(kind, path, self.a.clone(), path_key(&self.a));
        item.game_dir = Some(checked_path(&self.a).unwrap());
        item
    }
    fn plan(
        &self,
        items: &[Item],
        action: Action,
        parent: Option<&Path>,
        name: &str,
    ) -> operations::Plan {
        operations::prepare(
            &self.policy,
            items,
            action,
            parent,
            name,
            Conflict::Abort,
            VersionScope::CoreOnly,
            &AtomicBool::new(false),
        )
        .unwrap()
    }
}
#[test]
fn copy_is_verified_preserves_source_and_updates_scanned_membership() {
    let f = Fixture::new();
    let world = f.world("世界 甲");
    let before = operations::snapshot(&world.path, &AtomicBool::new(false)).unwrap();
    let plan = f.plan(
        std::slice::from_ref(&world),
        Action::Copy,
        Some(&f.b.join("saves")),
        "",
    );
    let report =
        operations::execute(&f.policy, plan, &AtomicBool::new(false), |_, _, _| {}).unwrap();
    assert!(report.finished);
    assert_eq!(
        operations::snapshot(&world.path, &AtomicBool::new(false)).unwrap(),
        before
    );
    assert_eq!(
        operations::snapshot(&f.b.join("saves/世界 甲"), &AtomicBool::new(false)).unwrap(),
        before
    );
    let scanned = scanner::scan(&f.settings, &AtomicBool::new(false), |_| {});
    assert_eq!(
        scanned
            .items
            .iter()
            .filter(|i| i.kind == ItemKind::World)
            .count(),
        2
    );
}
#[test]
fn rename_preserves_world_contents_and_is_recorded() {
    let f = Fixture::new();
    let world = f.world("old");
    write(
        &world.path.join("session.lock"),
        b"closed-world runtime lock",
    );
    let plan = f.plan(std::slice::from_ref(&world), Action::Rename, None, "新世界");
    let report =
        operations::execute(&f.policy, plan, &AtomicBool::new(false), |_, _, _| {}).unwrap();
    assert!(report.finished);
    assert!(!world.path.exists());
    assert_eq!(
        fs::read(f.a.join("saves/新世界/level.dat")).unwrap(),
        b"fixture world"
    );
    assert!(
        operations::history(&f.policy)
            .unwrap()
            .contains("重命名完成")
    );
}
#[test]
fn conflicts_are_rejected_or_resolved_without_overwriting() {
    let f = Fixture::new();
    let world = f.world("same");
    write(&f.b.join("saves/same/keep.txt"), b"keep");
    assert!(
        operations::prepare(
            &f.policy,
            std::slice::from_ref(&world),
            Action::Copy,
            Some(&f.b.join("saves")),
            "",
            Conflict::Abort,
            VersionScope::CoreOnly,
            &AtomicBool::new(false)
        )
        .is_err()
    );
    let plan = operations::prepare(
        &f.policy,
        &[world],
        Action::Copy,
        Some(&f.b.join("saves")),
        "",
        Conflict::KeepBoth,
        VersionScope::CoreOnly,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(
        plan.units[0]
            .destination
            .as_ref()
            .unwrap()
            .ends_with("same (1)")
    );
    assert!(
        operations::execute(&f.policy, plan, &AtomicBool::new(false), |_, _, _| {})
            .unwrap()
            .finished
    );
    assert_eq!(fs::read(f.b.join("saves/same/keep.txt")).unwrap(), b"keep");
}
#[test]
fn destination_created_after_preview_is_never_overwritten() {
    let f = Fixture::new();
    let world = f.world("source");
    let plan = f.plan(&[world], Action::Copy, Some(&f.b.join("saves")), "");
    write(&f.b.join("saves/source/keep"), b"concurrent data");
    let report =
        operations::execute(&f.policy, plan, &AtomicBool::new(false), |_, _, _| {}).unwrap();
    assert!(!report.finished);
    assert_eq!(
        fs::read(f.b.join("saves/source/keep")).unwrap(),
        b"concurrent data"
    );
    assert!(f.a.join("saves/source").exists());
}
#[test]
fn source_changed_after_confirmation_is_rejected() {
    let f = Fixture::new();
    let world = f.world("source");
    let plan = f.plan(std::slice::from_ref(&world), Action::Rename, None, "new");
    write(&world.path.join("level.dat"), b"new state");
    let report =
        operations::execute(&f.policy, plan, &AtomicBool::new(false), |_, _, _| {}).unwrap();
    assert!(!report.finished);
    assert!(world.path.exists());
    assert!(!f.a.join("saves/new").exists());
}
#[test]
fn cancellation_cleans_staging_and_keeps_original() {
    let f = Fixture::new();
    let world = f.world("source");
    write(&world.path.join("large.bin"), &vec![9; 1024 * 1024]);
    let plan = f.plan(
        std::slice::from_ref(&world),
        Action::Copy,
        Some(&f.b.join("saves")),
        "",
    );
    let flag = AtomicBool::new(false);
    let report = operations::execute(&f.policy, plan, &flag, |_, _, _| {
        flag.store(true, Ordering::SeqCst);
    })
    .unwrap();
    assert!(!report.finished);
    assert!(world.path.exists());
    assert!(!f.b.join("saves/source").exists());
    assert_eq!(fs::read_dir(f.b.join("saves")).unwrap().count(), 0);
}
#[test]
fn batches_report_completed_items_and_stop_on_failure() {
    let f = Fixture::new();
    let a = f.world("a");
    let b = f.world("b");
    let plan = f.plan(&[a, b.clone()], Action::Copy, Some(&f.b.join("saves")), "");
    write(&b.path.join("changed.txt"), b"changed");
    let report =
        operations::execute(&f.policy, plan, &AtomicBool::new(false), |_, _, _| {}).unwrap();
    assert!(!report.finished);
    assert!(report.outcomes[0].success);
    assert!(!report.outcomes[1].success);
    assert!(f.b.join("saves/a/level.dat").exists());
    assert!(!f.b.join("saves/b").exists());
}
#[test]
fn backup_round_trip_has_manifest_and_rejects_tampered_payload() {
    let f = Fixture::new();
    let world = f.world("备份世界");
    let plan = f.plan(&[world], Action::Backup, Some(&f.backup), "");
    let backup = plan.units[0].destination.clone().unwrap();
    assert!(
        operations::execute(&f.policy, plan, &AtomicBool::new(false), |_, _, _| {})
            .unwrap()
            .finished
    );
    assert!(backup.join("minecraft-explorer-backup.json").exists());
    let restore = operations::prepare_restore(
        &f.policy,
        &backup,
        &f.b.join("saves"),
        Conflict::Abort,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(
        operations::execute(&f.policy, restore, &AtomicBool::new(false), |_, _, _| {})
            .unwrap()
            .finished
    );
    assert_eq!(
        fs::read(f.b.join("saves/备份世界/level.dat")).unwrap(),
        b"fixture world"
    );
    write(&backup.join("content/备份世界/level.dat"), b"tampered");
    assert!(
        operations::prepare_restore(
            &f.policy,
            &backup,
            &f.a.join("saves"),
            Conflict::KeepBoth,
            &AtomicBool::new(false)
        )
        .is_err()
    );
}
#[test]
fn archive_backup_and_restore_keep_original_extension() {
    let f = Fixture::new();
    let path = f.a.join("mods/a.jar");
    write(&path, b"test jar bytes");
    let item = f.item(&path, ItemKind::Mod);
    let plan = f.plan(&[item], Action::Backup, Some(&f.backup), "");
    let backup = plan.units[0].destination.clone().unwrap();
    assert!(
        operations::execute(&f.policy, plan, &AtomicBool::new(false), |_, _, _| {})
            .unwrap()
            .finished
    );
    let restore = operations::prepare_restore(
        &f.policy,
        &backup,
        &f.b.join("mods"),
        Conflict::Abort,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(
        operations::execute(&f.policy, restore, &AtomicBool::new(false), |_, _, _| {})
            .unwrap()
            .finished
    );
    assert_eq!(fs::read(f.b.join("mods/a.jar")).unwrap(), b"test jar bytes");
}
#[test]
fn high_risk_paths_and_wrong_content_destinations_are_protected() {
    let f = Fixture::new();
    for path in [&f.a, &f.a.join("versions"), &f.a.join("saves"), &f.data] {
        let item = f.item(path, ItemKind::World);
        assert!(
            operations::prepare(
                &f.policy,
                &[item],
                Action::Recycle,
                None,
                "",
                Conflict::Abort,
                VersionScope::CoreOnly,
                &AtomicBool::new(false)
            )
            .is_err()
        );
    }
    let world = f.world("source");
    assert!(
        operations::prepare(
            &f.policy,
            std::slice::from_ref(&world),
            Action::Copy,
            Some(&f.b.join("mods")),
            "",
            Conflict::Abort,
            VersionScope::CoreOnly,
            &AtomicBool::new(false)
        )
        .is_err()
    );
    assert!(
        operations::prepare(
            &f.policy,
            &[world],
            Action::Backup,
            Some(&f.a.join("saves")),
            "",
            Conflict::Abort,
            VersionScope::CoreOnly,
            &AtomicBool::new(false)
        )
        .is_err()
    );
}
#[test]
fn version_core_preview_does_not_include_shared_or_independent_worlds() {
    let f = Fixture::new();
    let path = f.a.join("versions/isolated");
    write(&path.join("isolated.json"), b"{}");
    write(&path.join("isolated.jar"), b"jar");
    write(
        &path.join("unrelated-settings.json"),
        b"must preserve other settings",
    );
    write(
        &path.join("unrelated.jar"),
        b"must preserve unrelated archive",
    );
    write(&path.join("saves/world/level.dat"), b"world");
    let mut item = f.item(&path, ItemKind::Version);
    item.game_dir = Some(checked_path(&path).unwrap());
    let plan = f.plan(&[item.clone()], Action::Recycle, None, "");
    assert_eq!(plan.units.len(), 2);
    assert!(
        plan.units
            .iter()
            .all(|u| u.source.file_stem().unwrap() == "isolated")
    );
    assert!(
        plan.units
            .iter()
            .all(|u| u.source.is_file() && !u.whole_instance)
    );
    item.game_dir = None;
    assert!(
        operations::prepare(
            &f.policy,
            &[item],
            Action::Recycle,
            None,
            "",
            Conflict::Abort,
            VersionScope::IndependentDirectory,
            &AtomicBool::new(false)
        )
        .is_err()
    );
    assert!(path.join("saves/world/level.dat").exists());
}
#[test]
fn invalid_names_parent_child_selection_and_container_renames_are_rejected() {
    for name in [
        "",
        "..",
        "../evil",
        "CON",
        "nul.txt",
        "COM1.jar",
        "bad:name",
        "trailing.",
        "trailing ",
    ] {
        assert!(operations::valid_name(name).is_err(), "{name}");
    }
    let f = Fixture::new();
    let world = f.world("world");
    let data = world.path.join("datapacks/pack");
    write(&data.join("pack.mcmeta"), b"{}");
    write(&data.join("data/test/a.json"), b"{}");
    let item = f.item(&data, ItemKind::DataPack);
    assert!(
        operations::prepare(
            &f.policy,
            &[world, item],
            Action::Recycle,
            None,
            "",
            Conflict::Abort,
            VersionScope::CoreOnly,
            &AtomicBool::new(false)
        )
        .is_err()
    );
}
#[cfg(windows)]
#[test]
fn locked_world_is_rejected_without_touching_files() {
    use std::os::windows::fs::OpenOptionsExt;
    let f = Fixture::new();
    let world = f.world("locked");
    write(&world.path.join("session.lock"), b"running");
    let plan = f.plan(
        std::slice::from_ref(&world),
        Action::Rename,
        None,
        "renamed",
    );
    let _lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(world.path.join("session.lock"))
        .unwrap();
    assert!(operations::execute(&f.policy, plan, &AtomicBool::new(false), |_, _, _| {}).is_err());
    assert!(world.path.exists());
    assert!(!f.a.join("saves/renamed").exists());
}
#[cfg(windows)]
#[test]
fn junction_inside_world_prevents_mutation() {
    let f = Fixture::new();
    let world = f.world("linked");
    let external = f._temp.path().join("external");
    fs::create_dir(&external).unwrap();
    write(&external.join("keep"), b"keep");
    let status = std::process::Command::new("cmd.exe")
        .args(["/C", "mklink", "/J"])
        .arg(world.path.join("outside"))
        .arg(&external)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(
        operations::prepare(
            &f.policy,
            &[world],
            Action::Recycle,
            None,
            "",
            Conflict::Abort,
            VersionScope::CoreOnly,
            &AtomicBool::new(false)
        )
        .is_err()
    );
    assert_eq!(fs::read(external.join("keep")).unwrap(), b"keep");
}
#[test]
fn recycle_guard_vetoes_permanent_delete_flags() {
    assert!(!minecraft_explorer::recycle::recycle_flags_allowed(0));
    assert!(minecraft_explorer::recycle::recycle_flags_allowed(0x80));
}
#[test]
fn rename_annotation_sync_preserves_favorites_tags_recent_and_descendants() {
    let f = Fixture::new();
    let world = f.world("old");
    let data = world.path.join("datapacks/pack");
    write(&data.join("pack.mcmeta"), b"{}");
    write(&data.join("data/a.json"), b"{}");
    let child = f.item(&data, ItemKind::DataPack);
    let index = Index::open(&f.data.join("index.db")).unwrap();
    index.upsert(&world, 1).unwrap();
    index.upsert(&child, 1).unwrap();
    index
        .annotate(&world.id, true, &["世界标签".into()])
        .unwrap();
    index.record_opened(&world.id).unwrap();
    index.annotate(&child.id, true, &["子标签".into()]).unwrap();
    let target = f.a.join("saves/new");
    index.relocate_annotations(&world.path, &target).unwrap();
    let mut new = world.clone();
    new.path = target.clone();
    new.id = format!("world:{}", path_key(&target));
    index.upsert(&new, 2).unwrap();
    let item = index.get(&new.id).unwrap().unwrap();
    assert!(item.favorite);
    assert_eq!(item.tags, ["世界标签"]);
    assert!(item.last_opened_ms.is_some());
    let mut new_child = child;
    new_child.path = target.join("datapacks/pack");
    new_child.id = format!("data_pack:{}", path_key(&new_child.path));
    index.upsert(&new_child, 2).unwrap();
    assert_eq!(index.get(&new_child.id).unwrap().unwrap().tags, ["子标签"]);
}

#[cfg(windows)]
#[test]
#[ignore = "Explicit native recycle-bin acceptance on generated temporary files only"]
fn native_recycle_and_verified_move_are_recoverable() {
    fn recycle_pair(source: &Path) -> (PathBuf, PathBuf) {
        let original = minecraft_explorer::platform::display_path(source);
        let bin = PathBuf::from(format!("{}\\$Recycle.Bin", &original[..2]));
        for sid in fs::read_dir(&bin).unwrap().flatten() {
            let Ok(entries) = fs::read_dir(sid.path()) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.starts_with("$I") {
                    continue;
                }
                let Ok(bytes) = fs::read(entry.path()) else {
                    continue;
                };
                if bytes.len() < 28 {
                    continue;
                }
                let version = u64::from_le_bytes(bytes[..8].try_into().unwrap());
                let start = if version == 2 {
                    28
                } else if version == 1 {
                    24
                } else {
                    continue;
                };
                let units = bytes[start..]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .take_while(|c| *c != 0)
                    .collect::<Vec<_>>();
                if path_key(Path::new(&String::from_utf16_lossy(&units))) == path_key(source) {
                    return (
                        entry.path(),
                        entry.path().with_file_name(format!("$R{}", &name[2..])),
                    );
                }
            }
        }
        panic!("source is missing but no matching recycle metadata exists: {original}");
    }
    let f = Fixture::new();
    let world = f.world("native-recycle-world");
    write(&world.path.join("session.lock"), b"fixture closed lock");
    {
        use std::os::windows::fs::OpenOptionsExt;
        let active = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(7)
            .open(world.path.join("session.lock"))
            .unwrap();
        active.try_lock().unwrap();
        assert!(
            minecraft_explorer::recycle::send(&world.path, &AtomicBool::new(false)).is_err(),
            "Shell must refuse a real exclusive session lock"
        );
        assert_eq!(
            fs::read(world.path.join("level.dat")).unwrap(),
            b"fixture world"
        );
    }
    let plan = f.plan(std::slice::from_ref(&world), Action::Recycle, None, "");
    let report =
        operations::execute(&f.policy, plan, &AtomicBool::new(false), |_, _, _| {}).unwrap();
    assert!(report.finished, "{report:?}");
    assert!(!world.path.exists());
    let (metadata, recycled) = recycle_pair(&world.path);
    assert_eq!(
        fs::read(recycled.join("level.dat")).unwrap(),
        b"fixture world"
    );
    // 仅恢复本测试生成的确切对象，再移除对应 Shell 记录。
    fs::rename(&recycled, &world.path).unwrap();
    fs::remove_file(metadata).unwrap();
    assert!(world.path.join("level.dat").exists());
    let jar = f.a.join("mods/native-move.jar");
    write(&jar, b"native verified move fixture");
    let item = f.item(&jar, ItemKind::Mod);
    let plan = f.plan(&[item], Action::Move, Some(&f.b.join("mods")), "");
    let report =
        operations::execute(&f.policy, plan, &AtomicBool::new(false), |_, _, _| {}).unwrap();
    assert!(report.finished, "{report:?}");
    assert!(!jar.exists());
    assert_eq!(
        fs::read(f.b.join("mods/native-move.jar")).unwrap(),
        b"native verified move fixture"
    );
    let (metadata, recycled) = recycle_pair(&jar);
    assert_eq!(
        fs::read(&recycled).unwrap(),
        b"native verified move fixture"
    );
    fs::rename(recycled, &jar).unwrap();
    fs::remove_file(metadata).unwrap();

    // 在本机 C: 与 E: 卷之间验证同样的移动流程。
    let artifact = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".test-artifacts");
    fs::create_dir_all(&artifact).unwrap();
    let cross = tempfile::tempdir_in(&artifact).unwrap();
    let target_game = cross.path().join("cross-volume-Minecraft");
    fs::create_dir_all(target_game.join("mods")).unwrap();
    let mut settings = f.settings.clone();
    settings.sources.push(Source {
        path: target_game.clone(),
        kind: SourceKind::Root,
        associated_root: None,
    });
    let mut game = base_item(
        ItemKind::Instance,
        target_game.clone(),
        target_game.clone(),
        path_key(&target_game),
    );
    game.game_dir = Some(target_game.clone());
    let mut source_game = base_item(ItemKind::Instance, f.a.clone(), f.a.clone(), path_key(&f.a));
    source_game.game_dir = Some(f.a.clone());
    let policy = Policy::new(&settings, &[game, source_game], &f.data).unwrap();
    let item = f.item(&jar, ItemKind::Mod);
    let plan = operations::prepare(
        &policy,
        &[item],
        Action::Move,
        Some(&target_game.join("mods")),
        "",
        Conflict::Abort,
        VersionScope::CoreOnly,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(
        operations::execute(&policy, plan, &AtomicBool::new(false), |_, _, _| {})
            .unwrap()
            .finished
    );
    assert_eq!(
        fs::read(target_game.join("mods/native-move.jar")).unwrap(),
        b"native verified move fixture"
    );
    let (metadata, recycled) = recycle_pair(&jar);
    fs::rename(recycled, &jar).unwrap();
    fs::remove_file(metadata).unwrap();

    let version = f.a.join("versions/native-core");
    write(&version.join("native-core.json"), b"{}");
    write(&version.join("native-core.jar"), b"core jar");
    write(&version.join("saves/preserved/level.dat"), b"must stay");
    let item = f.item(&version, ItemKind::Version);
    let plan = f.plan(&[item], Action::Recycle, None, "");
    assert!(
        operations::execute(&f.policy, plan, &AtomicBool::new(false), |_, _, _| {})
            .unwrap()
            .finished
    );
    assert_eq!(
        fs::read(version.join("saves/preserved/level.dat")).unwrap(),
        b"must stay"
    );
    for name in ["native-core.json", "native-core.jar"] {
        let source = checked_path(&version).unwrap().join(name);
        let (metadata, recycled) = recycle_pair(&source);
        fs::rename(recycled, source).unwrap();
        fs::remove_file(metadata).unwrap();
    }
    let mut independent = f.item(&version, ItemKind::Version);
    independent.game_dir = Some(checked_path(&version).unwrap());
    let mut game = base_item(
        ItemKind::Instance,
        checked_path(&version).unwrap(),
        f.a.clone(),
        path_key(&f.a),
    );
    game.game_dir = Some(checked_path(&version).unwrap());
    let policy = Policy::new(&f.settings, &[game], &f.data).unwrap();
    let plan = operations::prepare(
        &policy,
        &[independent],
        Action::Recycle,
        None,
        "",
        Conflict::Abort,
        VersionScope::IndependentDirectory,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(plan.units[0].whole_instance);
    assert!(
        operations::execute(&policy, plan, &AtomicBool::new(false), |_, _, _| {})
            .unwrap()
            .finished
    );
    assert!(!version.exists());
    let (metadata, recycled) = recycle_pair(&version);
    assert_eq!(
        fs::read(recycled.join("saves/preserved/level.dat")).unwrap(),
        b"must stay"
    );
    fs::rename(recycled, &version).unwrap();
    fs::remove_file(metadata).unwrap();
}

#[test]
fn removing_source_registration_invalidates_prepared_operation() {
    let f = Fixture::new();
    let world = f.world("source");
    let plan = f.plan(std::slice::from_ref(&world), Action::Rename, None, "new");
    let policy = Policy::new(&Settings::default(), &[], &f.data).unwrap();
    assert!(operations::execute(&policy, plan, &AtomicBool::new(false), |_, _, _| {}).is_err());
    assert!(world.path.exists());
}

#[test]
fn old_temp_stages_are_excluded_from_world_discovery_and_shown_in_history() {
    let f = Fixture::new();
    let world = f.world("source");
    let plan = f.plan(&[world], Action::Copy, Some(&f.b.join("saves")), "");
    let stage =
        f.b.join("saves")
            .join(format!(".minecraft-explorer-{}-0.partial", plan.id));
    write(&stage.join("payload/level.dat"), b"interrupted copy");
    fs::create_dir_all(&f.policy.records).unwrap();
    let report = operations::Report {
        plan: plan.clone(),
        outcomes: Vec::new(),
        finished: false,
    };
    write(
        &f.policy.records.join(format!("{}.json", plan.id)),
        &serde_json::to_vec(&report).unwrap(),
    );
    let scan = scanner::scan(&f.settings, &AtomicBool::new(false), |_| {});
    assert!(
        !scan
            .items
            .iter()
            .any(|i| i.kind == ItemKind::World && i.folder.starts_with(".minecraft-explorer-"))
    );
    let history = operations::history(&f.policy).unwrap();
    assert!(history.contains("未清理临时副本") && history.contains("计划源"));
    assert!(stage.exists());
}

#[cfg(windows)]
#[test]
fn denied_destination_keeps_source_and_reports_failure() {
    use std::os::windows::process::CommandExt;
    let f = Fixture::new();
    let world = f.world("source");
    let destination = f.b.join("saves");
    assert!(
        checked_path(&destination)
            .unwrap()
            .starts_with(checked_path(f._temp.path()).unwrap())
    );
    let plan = f.plan(
        std::slice::from_ref(&world),
        Action::Copy,
        Some(&destination),
        "",
    );
    let denied = std::process::Command::new("icacls.exe")
        .creation_flags(0x08000000)
        .arg(&destination)
        .args(["/deny", "*S-1-1-0:(OI)(CI)(W)"])
        .output()
        .unwrap();
    assert!(denied.status.success());
    let result = operations::execute(&f.policy, plan, &AtomicBool::new(false), |_, _, _| {});
    let restored = std::process::Command::new("icacls.exe")
        .creation_flags(0x08000000)
        .arg(&destination)
        .args(["/remove:d", "*S-1-1-0"])
        .output()
        .unwrap();
    assert!(restored.status.success());
    if let Ok(report) = result {
        assert!(!report.finished);
    }
    assert!(world.path.join("level.dat").exists());
    assert!(!destination.join("source").exists());
    assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
}

#[test]
fn changing_source_during_copy_discards_uncommitted_destination() {
    let f = Fixture::new();
    let source = f.a.join("mods/change.jar");
    write(&source, &vec![7; 1024 * 1024]);
    let item = f.item(&source, ItemKind::Mod);
    let plan = f.plan(&[item], Action::Copy, Some(&f.b.join("mods")), "");
    let mut changed = false;
    let report = operations::execute(&f.policy, plan, &AtomicBool::new(false), |_, _, _| {
        if !changed {
            write(&source, b"new externally saved state");
            changed = true;
        }
    })
    .unwrap();
    assert!(!report.finished);
    assert_eq!(fs::read(source).unwrap(), b"new externally saved state");
    assert_eq!(fs::read_dir(f.b.join("mods")).unwrap().count(), 0);
}
