use minecraft_explorer::{content, models::ItemKind};
use std::{fs, io::Write, path::Path};

fn archive(path: &Path, entries: &[(&str, &[u8])]) {
    let mut writer = zip::ZipWriter::new(fs::File::create(path).unwrap());
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in entries {
        writer.start_file(*name, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
}

fn png(width: u32, height: u32) -> Vec<u8> {
    use image::ImageEncoder;
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(
            &vec![128; width as usize * height as usize * 4],
            width,
            height,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
    bytes
}

#[test]
fn resource_archive_preserves_component_description_and_format_declarations() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("资源包.zip");
    let metadata = br#"{"pack":{"pack_format":15,"supported_formats":{"min_inclusive":15,"max_inclusive":18},"description":{"text":"Textures ","extra":[{"text":"by Ada"}]}}}"#;
    let icon = png(16, 16);
    archive(
        &path,
        &[
            ("pack.mcmeta", metadata),
            ("pack.png", &icon),
            ("assets/ignored.bin", b"unread"),
        ],
    );
    let report = content::metadata(&path, &ItemKind::ResourcePack);
    assert_eq!(
        report.metadata.description.as_deref(),
        Some("Textures by Ada")
    );
    assert_eq!(report.metadata.compatibility.len(), 2);
    assert!(report.metadata.compatibility[1].contains("min_inclusive"));
    assert!(report.status.contains("已读取"));
    assert!(report.display_name.is_none());
    assert!(report.metadata.declared_version.is_none());
    assert_eq!(content::preview(&path, &ItemKind::ResourcePack), Some(icon));
    assert_eq!(
        fs::read_dir(temp.path()).unwrap().count(),
        1,
        "preview never extracts files"
    );
}

#[test]
fn directory_data_pack_reads_modern_major_minor_fields_and_translation_fallback() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("pack.mcmeta"), r#"{"pack":{"description":{"translate":"pack.example","fallback":"Example","extra":[{"text":" data"}]},"min_format":[88,0],"max_format":[88,1]}}"#).unwrap();
    fs::write(temp.path().join("pack.png"), png(8, 8)).unwrap();
    let report = content::metadata(temp.path(), &ItemKind::DataPack);
    assert_eq!(report.metadata.description.as_deref(), Some("Example data"));
    assert_eq!(
        report.metadata.compatibility,
        ["min_format = [88,0]", "max_format = [88,1]"]
    );
    assert!(content::preview(temp.path(), &ItemKind::DataPack).is_some());
}

#[test]
fn fabric_reads_display_name_authors_dependencies_and_resolution_icon() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("not-a-version.jar");
    let metadata = br#"{"schemaVersion":1,"id":"example_mod","version":"2.3.0","name":"Example Mod","description":"A sample","authors":["Ada",{"name":"Lin"}],"depends":{"minecraft":["1.21.x","1.22.x"],"fabricloader":">=0.15","java":">=17"},"icon":{"16":"assets/small.png","128":"assets/main.png"}}"#;
    let main = png(128, 128);
    let small = png(16, 16);
    archive(
        &path,
        &[
            ("fabric.mod.json", metadata),
            ("assets/main.png", &main),
            ("assets/small.png", &small),
        ],
    );
    let report = content::metadata(&path, &ItemKind::Mod);
    assert_eq!(report.display_name.as_deref(), Some("Example Mod"));
    assert_eq!(report.metadata.declared_id.as_deref(), Some("example_mod"));
    assert_eq!(report.metadata.declared_version.as_deref(), Some("2.3.0"));
    assert_eq!(report.metadata.authors, ["Ada", "Lin"]);
    assert_eq!(report.metadata.loaders, ["Fabric"]);
    assert_eq!(report.metadata.compatibility.len(), 3);
    assert_eq!(
        report.metadata.preview_entry.as_deref(),
        Some("assets/main.png")
    );
    assert_eq!(content::preview(&path, &ItemKind::Mod), Some(main));
}

#[test]
fn forge_multi_mod_preserves_each_name_version_description_and_dependency_owner() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("combined.jar");
    let metadata = br#"
modLoader = "javafml"
loaderVersion = "[47,)"
[[mods]]
modId = "alpha"
version = "1.0"
displayName = "Alpha"
description = "First mod"
authors = "Ada, Lin"
logoFile = "logo.png"
[[mods]]
modId = "beta"
version = "${file.jarVersion}"
displayName = "Beta"
description = "Second mod"
[[dependencies.alpha]]
modId = "minecraft"
versionRange = "[1.20.1,1.21)"
[[dependencies.beta]]
modId = "forge"
versionRange = "[47,)"
"#;
    archive(
        &path,
        &[("META-INF/mods.toml", metadata), ("logo.png", &png(16, 16))],
    );
    let report = content::metadata(&path, &ItemKind::Mod);
    assert_eq!(report.display_name.as_deref(), Some("Alpha; Beta"));
    assert_eq!(report.metadata.declared_id.as_deref(), Some("alpha; beta"));
    let versions = report.metadata.declared_version.unwrap();
    assert!(versions.contains("alpha: 1.0") && versions.contains("beta: ${file.jarVersion}"));
    let description = report.metadata.description.unwrap();
    assert!(description.contains("alpha: First mod") && description.contains("beta: Second mod"));
    assert_eq!(report.metadata.loaders, ["Forge"]);
    assert!(
        report
            .metadata
            .compatibility
            .iter()
            .any(|value| value.starts_with("alpha → minecraft"))
    );
    assert!(report.status.contains("占位符"));
    assert!(content::preview(&path, &ItemKind::Mod).is_some());
}

#[test]
fn neoforge_and_legacy_forge_metadata_are_distinguished() {
    let temp = tempfile::tempdir().unwrap();
    let neo = temp.path().join("neo.jar");
    archive(&neo, &[("META-INF/neoforge.mods.toml", b"modLoader='javafml'\nloaderVersion='[4,)'\n[[mods]]\nmodId='neo_example'\nversion='1.2'\ndisplayName='Neo Example'\n")]);
    let report = content::metadata(&neo, &ItemKind::Mod);
    assert_eq!(report.metadata.loaders, ["NeoForge"]);
    assert_eq!(report.metadata.declared_version.as_deref(), Some("1.2"));
    let old = temp.path().join("old.jar");
    archive(&old, &[("mcmod.info", br#"{"modList":[{"modid":"old_mod","name":"Old Mod","version":"0.5","mcversion":"1.12.2","authorList":["Ada"]}]}"#)]);
    let report = content::metadata(&old, &ItemKind::Mod);
    assert_eq!(report.display_name.as_deref(), Some("Old Mod"));
    assert_eq!(report.metadata.loaders, ["Forge（mcmod.info）"]);
    assert_eq!(report.metadata.compatibility, ["minecraft: 1.12.2"]);
}

#[test]
fn unknown_and_corrupt_archives_keep_unknown_version_without_filename_guessing() {
    let temp = tempfile::tempdir().unwrap();
    let unknown = temp.path().join("forge-1.21.9-huge-version.jar");
    archive(
        &unknown,
        &[("META-INF/MANIFEST.MF", b"Manifest-Version: 1.0\r\n")],
    );
    let report = content::metadata(&unknown, &ItemKind::Mod);
    assert!(report.metadata.declared_id.is_none());
    assert!(report.metadata.declared_version.is_none());
    assert!(report.metadata.loaders.is_empty());
    assert!(report.status.contains("未知"));
    fs::write(&unknown, b"PK broken data").unwrap();
    let report = content::metadata(&unknown, &ItemKind::Mod);
    assert!(report.status.contains("不可读取"));
    assert!(content::preview(&unknown, &ItemKind::Mod).is_none());
}

#[test]
fn multiple_formats_in_one_jar_keep_all_explicit_loader_declarations() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("multiloader.jar");
    archive(&path, &[("fabric.mod.json", br#"[{"schemaVersion":1,"id":"alpha","version":"1","name":"Alpha"},{"schemaVersion":1,"id":"beta","version":"2","name":"Beta"}]"#),
        ("META-INF/neoforge.mods.toml", b"[[mods]]\nmodId='gamma'\nversion='3'\ndisplayName='Gamma'\n")]);
    let report = content::metadata(&path, &ItemKind::Mod);
    assert_eq!(report.metadata.loaders, ["Fabric", "NeoForge"]);
    assert_eq!(
        report.metadata.declared_id.as_deref(),
        Some("alpha; beta; gamma")
    );
    assert_eq!(report.display_name.as_deref(), Some("Alpha; Beta; Gamma"));
    assert!(
        report
            .metadata
            .declared_version
            .unwrap()
            .contains("beta: 2")
    );
}

#[test]
fn oversized_metadata_and_icons_are_rejected_before_unbounded_decompression() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("bomb.zip");
    let metadata = vec![b' '; 1024 * 1024 + 1];
    archive(
        &path,
        &[
            ("pack.mcmeta", &metadata),
            ("pack.png", &vec![0; 2 * 1024 * 1024 + 1]),
        ],
    );
    let report = content::metadata(&path, &ItemKind::ResourcePack);
    assert!(report.status.contains("上限"));
    assert!(report.metadata.description.is_none());
    assert!(content::preview(&path, &ItemKind::ResourcePack).is_none());
    fs::write(temp.path().join("pack.mcmeta"), &metadata).unwrap();
    assert!(
        content::metadata(temp.path(), &ItemKind::DataPack)
            .status
            .contains("上限")
    );
}

#[test]
fn unsafe_paths_encryption_and_duplicate_key_entries_degrade_safely() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("unsafe.zip");
    archive(&path, &[("../pack.mcmeta", b"{}")]);
    assert!(
        content::metadata(&path, &ItemKind::ResourcePack)
            .status
            .contains("不安全")
    );

    archive(&path, &[("pack.mcmeta", b"{}"), ("evil.mcmeta", b"{}")]);
    let mut bytes = fs::read(&path).unwrap();
    let positions: Vec<_> = bytes
        .windows(11)
        .enumerate()
        .filter_map(|(position, name)| (name == b"evil.mcmeta").then_some(position))
        .collect();
    for position in positions {
        bytes[position..position + 11].copy_from_slice(b"pack.mcmeta");
    }
    fs::write(&path, &bytes).unwrap();
    assert!(
        content::metadata(&path, &ItemKind::ResourcePack)
            .status
            .contains("重复")
    );

    archive(&path, &[("pack.mcmeta", b"{}")]);
    let mut bytes = fs::read(&path).unwrap();
    let central = bytes
        .windows(4)
        .position(|bytes| bytes == b"PK\x01\x02")
        .unwrap();
    bytes[central + 8] |= 1;
    fs::write(&path, &bytes).unwrap();
    assert!(
        content::metadata(&path, &ItemKind::ResourcePack)
            .status
            .contains("加密")
    );
}

#[test]
fn malicious_declared_icon_and_giant_png_dimensions_are_not_previewed() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("mod.jar");
    archive(
        &path,
        &[(
            "fabric.mod.json",
            br#"{"id":"example","version":"1","icon":"../outside.png"}"#,
        )],
    );
    let report = content::metadata(&path, &ItemKind::Mod);
    assert!(report.metadata.preview_entry.is_none());
    assert!(report.status.contains("不安全"));
    fs::write(temp.path().join("pack.png"), png(1025, 1)).unwrap();
    assert!(content::preview(temp.path(), &ItemKind::ShaderPack).is_none());
}

#[test]
fn malformed_toml_does_not_discard_other_valid_mod_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("partial.jar");
    archive(
        &path,
        &[
            (
                "fabric.mod.json",
                br#"{"id":"valid_mod","version":"1.0","name":"Valid"}"#,
            ),
            ("META-INF/mods.toml", b"[[mods]\nnotvalid"),
        ],
    );
    let report = content::metadata(&path, &ItemKind::Mod);
    assert_eq!(report.metadata.declared_id.as_deref(), Some("valid_mod"));
    assert!(report.status.contains("已读取") && report.status.contains("无法解析"));
}

#[test]
fn central_directory_entry_count_and_length_are_checked_before_allocating_zip_index() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("oversized-index.zip");
    archive(&path, &[("pack.mcmeta", b"{}")]);
    let original = fs::read(&path).unwrap();
    let end = original
        .windows(4)
        .rposition(|bytes| bytes == b"PK\x05\x06")
        .unwrap();
    let mut bytes = original.clone();
    bytes[end + 8..end + 10].copy_from_slice(&60000u16.to_le_bytes());
    bytes[end + 10..end + 12].copy_from_slice(&60000u16.to_le_bytes());
    fs::write(&path, &bytes).unwrap();
    assert!(
        content::metadata(&path, &ItemKind::ResourcePack)
            .status
            .contains("安全上限")
    );
    let mut bytes = original;
    bytes[end + 12..end + 16].copy_from_slice(&(9 * 1024 * 1024u32).to_le_bytes());
    fs::write(&path, &bytes).unwrap();
    assert!(
        content::metadata(&path, &ItemKind::ResourcePack)
            .status
            .contains("安全上限")
    );
}

#[test]
fn bounded_zip64_directory_is_supported_without_extracting() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("zip64.zip");
    archive(
        &path,
        &[(
            "pack.mcmeta",
            br#"{"pack":{"description":"ZIP64 pack","pack_format":15}}"#,
        )],
    );
    let original = fs::read(&path).unwrap();
    let end = original
        .windows(4)
        .rposition(|bytes| bytes == b"PK\x05\x06")
        .unwrap();
    let directory_size = u32::from_le_bytes(original[end + 12..end + 16].try_into().unwrap());
    let directory_start = u32::from_le_bytes(original[end + 16..end + 20].try_into().unwrap());
    let mut bytes = original[..end].to_vec();
    bytes.extend_from_slice(b"PK\x06\x06");
    bytes.extend_from_slice(&44u64.to_le_bytes());
    bytes.extend_from_slice(&45u16.to_le_bytes());
    bytes.extend_from_slice(&45u16.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&1u64.to_le_bytes());
    bytes.extend_from_slice(&1u64.to_le_bytes());
    bytes.extend_from_slice(&u64::from(directory_size).to_le_bytes());
    bytes.extend_from_slice(&u64::from(directory_start).to_le_bytes());
    bytes.extend_from_slice(b"PK\x06\x07");
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&(end as u64).to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    let mut footer = original[end..].to_vec();
    footer[8..12].fill(255);
    footer[12..20].fill(255);
    bytes.extend_from_slice(&footer);
    fs::write(&path, bytes).unwrap();
    let report = content::metadata(&path, &ItemKind::ResourcePack);
    assert_eq!(
        report.metadata.description.as_deref(),
        Some("ZIP64 pack"),
        "{}",
        report.status
    );
}

#[cfg(windows)]
#[test]
fn directory_pack_does_not_follow_junction_metadata() {
    use std::process::Command;
    let temp = tempfile::tempdir().unwrap();
    let pack = temp.path().join("pack");
    let outside = temp.path().join("outside");
    fs::create_dir(&pack).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(
        pack.join("fabric.mod.json"),
        br#"{"id":"example","version":"1","icon":"linked/icon.png"}"#,
    )
    .unwrap();
    fs::write(outside.join("icon.png"), png(16, 16)).unwrap();
    let junction = pack.join("linked");
    let quote = |path: &Path| format!("'{}'", path.display().to_string().replace('\'', "''"));
    let result = Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command",
        &format!("$ErrorActionPreference='Stop'; New-Item -ItemType Junction -Path {} -Target {} | Out-Null", quote(&junction), quote(&outside))])
        .output().unwrap();
    assert!(
        result.status.success(),
        "junction fixture: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let result = content::preview(&pack, &ItemKind::Mod);
    fs::remove_dir(junction).unwrap();
    assert!(result.is_none());
}
