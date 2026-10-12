//! 以有界、只读方式读取本地 Minecraft 内容元信息与 PNG 预览。
//! 不解压归档，不推断声明的兼容性是否实际成立。

use crate::{
    models::{ContentMetadata, ItemKind},
    platform::{checked_path, read_bounded},
};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

const ENTRY_LIMIT: u64 = 1024 * 1024;
const METADATA_BUDGET: u64 = 4 * 1024 * 1024;
const ICON_LIMIT: u64 = 2 * 1024 * 1024;
const DIRECTORY_LIMIT: u64 = 8 * 1024 * 1024;
const MAX_ENTRIES: u64 = 40_000;
const MAX_RECORDS: usize = 256;

#[derive(Clone, Debug)]
pub struct ContentMetadataReport {
    pub metadata: ContentMetadata,
    pub status: String,
    /// 仅在元信息文件明确声明显示名称时提供。
    pub display_name: Option<String>,
}

/// 仅解析已识别的元信息；损坏归档仍保留在列表中。
pub fn metadata(path: &Path, kind: &ItemKind) -> ContentMetadataReport {
    match ContentReader::open(path, METADATA_BUDGET) {
        Ok(mut reader) => parse(&mut reader, kind),
        Err(error) => ContentMetadataReport {
            metadata: ContentMetadata::default(),
            display_name: None,
            status: format!("元信息不可读取：{error}"),
        },
    }
}

/// 数据包必须包含根级元信息文件和 data 目录。
/// ZIP 工具常省略目录记录，因此安全的 data/... 条目
/// 也能证明 data 目录存在；不搜索嵌套的包根目录。
pub fn is_data_pack(path: &Path) -> Result<bool> {
    let mut reader = ContentReader::open(path, ENTRY_LIMIT)?;
    if reader.read("pack.mcmeta", ENTRY_LIMIT)?.is_none() {
        return Ok(false);
    }
    Ok(match &reader.container {
        Container::Directory(root) => match fs::symlink_metadata(root.join("data")) {
            Ok(_) => checked_path(&root.join("data"))?.is_dir(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(e.into()),
        },
        Container::Archive(archive) => archive
            .file_names()
            .any(|name| name == "data/" || name.starts_with("data/")),
    })
}

/// 读取包根目录的小 PNG 或模组明确声明的图标。
/// 不创建缓存文件，不跟随目录包中的链接。
pub fn preview(path: &Path, kind: &ItemKind) -> Option<Vec<u8>> {
    let mut reader = ContentReader::open(path, METADATA_BUDGET + ICON_LIMIT).ok()?;
    let report = parse(&mut reader, kind);
    let entry = report.metadata.preview_entry?;
    let bytes = reader.read(&entry, ICON_LIMIT).ok()??;
    if bytes.len() < 33
        || &bytes[..8] != b"\x89PNG\r\n\x1a\n"
        || bytes[8..12] != [0, 0, 0, 13]
        || &bytes[12..16] != b"IHDR"
    {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    if width == 0 || height == 0 || width > 1024 || height > 1024 {
        return None;
    }
    Some(bytes)
}

enum Container {
    Directory(PathBuf),
    Archive(zip::ZipArchive<File>),
}

struct ContentReader {
    container: Container,
    remaining: u64,
}

impl ContentReader {
    fn open(path: &Path, remaining: u64) -> Result<Self> {
        let path = checked_path(path)?;
        let metadata = fs::symlink_metadata(&path)?;
        let container = if metadata.is_dir() {
            Container::Directory(path)
        } else if metadata.is_file() {
            let mut file = File::open(&path)?;
            inspect_directory(&mut file)?;
            file.rewind()?;
            let config = zip::read::Config {
                archive_offset: zip::read::ArchiveOffset::Known(0),
            };
            Container::Archive(
                zip::ZipArchive::with_config(config, file)
                    .context("ZIP/JAR 结构损坏或格式不受支持")?,
            )
        } else {
            bail!("不是普通文件或目录");
        };
        Ok(Self {
            container,
            remaining,
        })
    }

    fn contains(&self, name: &str) -> bool {
        if !safe_entry(name.as_bytes()) || name.ends_with('/') {
            return false;
        }
        match &self.container {
            Container::Directory(root) => checked_path(&root.join(name))
                .ok()
                .is_some_and(|path| path.is_file()),
            Container::Archive(archive) => archive.file_names().any(|entry| entry == name),
        }
    }

    fn read(&mut self, name: &str, limit: u64) -> Result<Option<Vec<u8>>> {
        if !safe_entry(name.as_bytes()) || name.ends_with('/') {
            bail!("元信息中的归档路径不安全");
        }
        let limit = limit.min(self.remaining);
        let bytes = match &mut self.container {
            Container::Directory(root) => {
                let path = root.join(name);
                match fs::symlink_metadata(&path) {
                    Ok(_) => Some(read_bounded(&path, limit)?),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                    Err(error) => return Err(error.into()),
                }
            }
            Container::Archive(archive) => {
                let mut entry = match archive.by_name(name) {
                    Ok(entry) => entry,
                    Err(zip::result::ZipError::FileNotFound) => return Ok(None),
                    Err(error) => return Err(error).context("归档条目无法读取"),
                };
                if entry.encrypted()
                    || entry.is_dir()
                    || entry
                        .unix_mode()
                        .is_some_and(|mode| mode & 0o170000 == 0o120000)
                {
                    bail!("拒绝加密、目录或链接元信息条目");
                }
                if entry.size() > limit || entry.compressed_size() > limit.saturating_add(65536) {
                    bail!("元信息或图标超出读取上限");
                }
                let mut bytes = Vec::with_capacity(entry.size() as usize);
                (&mut entry)
                    .take(limit + 1)
                    .read_to_end(&mut bytes)
                    .context("归档内容损坏或校验失败")?;
                if bytes.len() as u64 > limit {
                    bail!("元信息或图标超出读取上限");
                }
                Some(bytes)
            }
        };
        if let Some(bytes) = &bytes {
            self.remaining = self.remaining.saturating_sub(bytes.len() as u64);
        }
        Ok(bytes)
    }
}

fn safe_entry(name: &[u8]) -> bool {
    !name.is_empty()
        && name.len() <= 1024
        && !name.starts_with(b"/")
        && !name.iter().any(|byte| matches!(byte, 0 | b'\\' | b':'))
        && name
            .split(|byte| *byte == b'/')
            .all(|part| part != b"." && part != b"..")
}

fn u16_le(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}
fn u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
fn u64_le(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

/// ZipArchive 会合并同名条目，并按声明的条目数
/// 分配索引；使用前先在预算内验证中央目录。
fn inspect_directory(file: &mut File) -> Result<()> {
    let file_size = file.metadata()?.len();
    let tail_size = file_size.min(65535 + 22);
    if tail_size < 22 {
        bail!("归档末尾记录缺失");
    }
    file.seek(SeekFrom::End(-(tail_size as i64)))?;
    let mut tail = vec![0; tail_size as usize];
    file.read_exact(&mut tail)?;
    let end = (0..=tail.len() - 22)
        .rev()
        .find(|&position| {
            &tail[position..position + 4] == b"PK\x05\x06"
                && position + 22 + usize::from(u16_le(&tail, position + 20)) == tail.len()
        })
        .context("ZIP 末尾记录损坏或包含尾随数据")?;
    let eocd_position = file_size - tail_size + end as u64;
    let eocd = &tail[end..];
    if u16_le(eocd, 4) != 0 || u16_le(eocd, 6) != 0 {
        bail!("不读取分卷归档");
    }
    let mut count = u64::from(u16_le(eocd, 10));
    let mut directory_size = u64::from(u32_le(eocd, 12));
    let mut directory_start = u64::from(u32_le(eocd, 16));
    let mut directory_end = eocd_position;
    let mut locator = [0; 20];
    let has_locator = if eocd_position >= 20 {
        file.seek(SeekFrom::Start(eocd_position - 20))?;
        file.read_exact(&mut locator)?;
        &locator[..4] == b"PK\x06\x07"
    } else {
        false
    };
    if has_locator {
        if u32_le(&locator, 4) != 0 || u32_le(&locator, 16) != 1 {
            bail!("不读取分卷 ZIP64 归档");
        }
        let zip64_position = u64_le(&locator, 8);
        if zip64_position
            .checked_add(56)
            .is_none_or(|value| value > eocd_position - 20)
        {
            bail!("ZIP64 记录越界");
        }
        file.seek(SeekFrom::Start(zip64_position))?;
        let mut header = [0; 56];
        file.read_exact(&mut header)?;
        let length = u64_le(&header, 4);
        if &header[..4] != b"PK\x06\x06"
            || !(44..=4096).contains(&length)
            || zip64_position.checked_add(12 + length) != Some(eocd_position - 20)
            || u32_le(&header, 16) != 0
            || u32_le(&header, 20) != 0
            || u64_le(&header, 24) != u64_le(&header, 32)
        {
            bail!("ZIP64 记录损坏或超出上限");
        }
        count = u64_le(&header, 32);
        directory_size = u64_le(&header, 40);
        directory_start = u64_le(&header, 48);
        directory_end = zip64_position;
    } else if u16_le(eocd, 8) != u16_le(eocd, 10)
        || count == 65535
        || directory_size == u64::from(u32::MAX)
        || directory_start == u64::from(u32::MAX)
    {
        bail!("归档条目计数异常或 ZIP64 记录缺失");
    }
    if count > MAX_ENTRIES || directory_size > DIRECTORY_LIMIT {
        bail!("归档目录超出安全上限（40000 条目 / 8 MiB）");
    }
    if directory_start.checked_add(directory_size) != Some(directory_end) {
        bail!("归档中央目录范围异常");
    }
    file.seek(SeekFrom::Start(directory_start))?;
    let mut directory = vec![0; directory_size as usize];
    file.read_exact(&mut directory)?;
    let mut position = 0;
    let mut names = HashSet::new();
    for _ in 0..count {
        let Some(header) = directory.get(position..position + 46) else {
            bail!("归档目录条目不完整");
        };
        if &header[..4] != b"PK\x01\x02" {
            bail!("归档目录条目损坏");
        }
        let name_length = usize::from(u16_le(header, 28));
        let length =
            46 + name_length + usize::from(u16_le(header, 30)) + usize::from(u16_le(header, 32));
        let Some(entry) = directory.get(position..position + length) else {
            bail!("归档目录条目越界");
        };
        let name = &entry[46..46 + name_length];
        if !safe_entry(name) {
            bail!("归档包含不安全路径条目");
        }
        if !names.insert(name.to_vec()) {
            bail!("归档包含重复路径条目，拒绝歧义元信息");
        }
        if u16_le(header, 34) != 0 {
            bail!("归档条目指向其他分卷");
        }
        if u16_le(header, 8) & 1 != 0 {
            bail!("不读取加密归档");
        }
        if u32_le(header, 38) >> 16 & 0o170000 == 0o120000 {
            bail!("归档包含链接条目");
        }
        position += length;
    }
    if position != directory.len() {
        bail!("归档目录计数或长度不一致");
    }
    Ok(())
}

#[derive(Default)]
struct Collected {
    metadata: ContentMetadata,
    names: Vec<String>,
    ids: Vec<String>,
    versions: Vec<String>,
    descriptions: Vec<String>,
    read_formats: Vec<String>,
    problems: Vec<String>,
}

fn bounded_text(value: &str, limit: usize) -> Option<String> {
    let text: String = value
        .chars()
        .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\t'))
        .take(limit)
        .collect();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}
fn text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .and_then(|value| bounded_text(value, 512))
}
fn add_unique(values: &mut Vec<String>, value: String) {
    if values.len() < MAX_RECORDS && !values.contains(&value) {
        values.push(value);
    }
}
fn joined(values: &[String]) -> Option<String> {
    bounded_text(&values.join("; "), 32768)
}

impl Collected {
    fn problem(&mut self, message: String) {
        if self.problems.len() < 16 {
            self.problems.push(message);
        }
    }
    fn record(
        &mut self,
        id: Option<String>,
        name: Option<String>,
        version: Option<String>,
        description: Option<String>,
        multi: bool,
    ) {
        if let Some(name) = name {
            add_unique(&mut self.names, name);
        }
        if let Some(version) = version {
            let value = if multi {
                format!("{}: {version}", id.as_deref().unwrap_or("未声明 ID"))
            } else {
                version
            };
            add_unique(&mut self.versions, value);
        }
        if let Some(description) = description {
            let value = if multi {
                format!("{}: {description}", id.as_deref().unwrap_or("未声明 ID"))
            } else {
                description
            };
            add_unique(&mut self.descriptions, value);
        }
        if let Some(id) = id {
            add_unique(&mut self.ids, id);
        }
    }
    fn finish(mut self, kind: &ItemKind) -> ContentMetadataReport {
        self.metadata.declared_id = joined(&self.ids);
        self.metadata.declared_version = joined(&self.versions);
        self.metadata.description = joined(&self.descriptions);
        let status = if self.read_formats.is_empty() && self.problems.is_empty() {
            match kind {
                ItemKind::Mod => "未发现支持的模组元信息；ID、版本和加载器未知",
                ItemKind::ShaderPack => "未发现标准包元信息；光影版本和兼容性未知",
                _ => "未发现 pack.mcmeta；包描述和兼容性未知",
            }
            .to_string()
        } else {
            let mut parts = Vec::new();
            if !self.read_formats.is_empty() {
                parts.push(format!("已读取 {}", self.read_formats.join("、")));
            }
            parts.extend(self.problems);
            parts.join("；")
        };
        ContentMetadataReport {
            metadata: self.metadata,
            status,
            display_name: joined(&self.names),
        }
    }
}

fn parse(reader: &mut ContentReader, kind: &ItemKind) -> ContentMetadataReport {
    let mut collected = Collected::default();
    if matches!(kind, ItemKind::Mod) {
        for name in [
            "fabric.mod.json",
            "META-INF/mods.toml",
            "META-INF/neoforge.mods.toml",
            "mcmod.info",
        ] {
            match reader.read(name, ENTRY_LIMIT) {
                Ok(Some(bytes)) => {
                    let parsed = match name {
                        "fabric.mod.json" => parse_fabric(&bytes, &mut collected),
                        "mcmod.info" => parse_legacy_forge(&bytes, &mut collected),
                        _ => parse_forge(
                            &bytes,
                            name == "META-INF/neoforge.mods.toml",
                            &mut collected,
                        ),
                    };
                    match parsed {
                        Ok(()) => collected.read_formats.push(name.to_string()),
                        Err(error) => collected.problem(format!("{name} 无法解析：{error}")),
                    }
                }
                Ok(None) => {}
                Err(error) => collected.problem(format!("{name} 不可读取：{error}")),
            }
        }
    } else if matches!(
        kind,
        ItemKind::ResourcePack | ItemKind::DataPack | ItemKind::ShaderPack
    ) {
        match reader.read("pack.mcmeta", ENTRY_LIMIT) {
            Ok(Some(bytes)) => match parse_pack(&bytes, &mut collected) {
                Ok(()) => collected.read_formats.push("pack.mcmeta".to_string()),
                Err(error) => collected.problem(format!("pack.mcmeta 无法解析：{error}")),
            },
            Ok(None) => {}
            Err(error) => collected.problem(format!("pack.mcmeta 不可读取：{error}")),
        }
        if reader.contains("pack.png") {
            collected.metadata.preview_entry = Some("pack.png".into());
        }
    } else {
        collected.problem("此类型不使用内容包元信息".to_string());
    }
    if let Some(entry) = &collected.metadata.preview_entry
        && (!safe_entry(entry.as_bytes()) || !entry.to_lowercase().ends_with(".png"))
    {
        collected.metadata.preview_entry = None;
        collected.problem("元信息图标路径不安全或不是 PNG，已忽略".to_string());
    }
    collected.finish(kind)
}

fn parse_pack(bytes: &[u8], collected: &mut Collected) -> Result<()> {
    let root: Value = serde_json::from_slice(bytes)?;
    let pack = root
        .get("pack")
        .and_then(Value::as_object)
        .context("缺少 pack 对象")?;
    if let Some(description) = pack.get("description") {
        let mut output = String::new();
        flatten_component(description, &mut output, 0);
        if let Some(description) = bounded_text(&output, 8192) {
            collected.descriptions.push(description);
        }
    }
    for field in [
        "pack_format",
        "supported_formats",
        "min_format",
        "max_format",
    ] {
        if let Some(value) = pack.get(field) {
            let rendered = serde_json::to_string(value)?;
            if rendered.len() <= 512 {
                collected
                    .metadata
                    .compatibility
                    .push(format!("{field} = {rendered}"));
            } else {
                collected.problem(format!("{field} 字段过长，未展示"));
            }
        }
    }
    Ok(())
}

fn flatten_component(value: &Value, output: &mut String, depth: usize) {
    if depth > 16 || output.len() >= 32768 {
        return;
    }
    match value {
        Value::String(value) => {
            append_component_text(output, value, 8192);
        }
        Value::Array(values) => {
            for value in values.iter().take(MAX_RECORDS) {
                flatten_component(value, output, depth + 1);
            }
        }
        Value::Object(values) => {
            if let Some(value) = values.get("text") {
                flatten_component(value, output, depth + 1);
            } else if let Some(key) = values.get("translate").and_then(Value::as_str) {
                if let Some(fallback) = values.get("fallback").and_then(Value::as_str) {
                    append_component_text(output, fallback, 8192);
                } else if let Some(key) = bounded_text(key, 512) {
                    output.push_str(&format!("[翻译键：{key}]"));
                }
                if let Some(values) = values.get("with").and_then(Value::as_array) {
                    for value in values.iter().take(MAX_RECORDS) {
                        output.push(' ');
                        flatten_component(value, output, depth + 1);
                    }
                }
            }
            if let Some(extra) = values.get("extra") {
                flatten_component(extra, output, depth + 1);
            }
        }
        _ => {}
    }
}

fn append_component_text(output: &mut String, value: &str, limit: usize) {
    for ch in value
        .chars()
        .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\t'))
        .take(limit)
    {
        if output.len() >= 32768 {
            break;
        }
        output.push(ch);
    }
}

fn json_records(root: &Value) -> Vec<&Value> {
    root.as_array().map_or_else(
        || vec![root],
        |array| array.iter().take(MAX_RECORDS).collect(),
    )
}

fn parse_fabric(bytes: &[u8], collected: &mut Collected) -> Result<()> {
    let root: Value = serde_json::from_slice(bytes)?;
    let records = json_records(&root);
    if records.is_empty() || !records.iter().all(|record| record.is_object()) {
        bail!("需要模组对象或对象数组");
    }
    add_unique(&mut collected.metadata.loaders, "Fabric".into());
    let multi = records.len() > 1;
    for record in records {
        let id = text(record.get("id"));
        let version = text(record.get("version"));
        if id.is_none() || version.is_none() {
            collected.problem("Fabric 模组未完整声明 ID 和版本".into());
        }
        let description = record
            .get("description")
            .and_then(Value::as_str)
            .and_then(|value| bounded_text(value, 8192));
        collected.record(id, text(record.get("name")), version, description, multi);
        if let Some(authors) = record.get("authors").and_then(Value::as_array) {
            for author in authors.iter().take(MAX_RECORDS) {
                if let Some(name) = author
                    .as_str()
                    .and_then(|value| bounded_text(value, 512))
                    .or_else(|| text(author.get("name")))
                {
                    add_unique(&mut collected.metadata.authors, name);
                }
            }
        }
        // Fabric 依赖范围按声明原样显示，不能当作
        // 已经通过兼容性或依赖解析验证。
        if let Some(depends) = record.get("depends").and_then(Value::as_object) {
            for field in ["minecraft", "fabricloader", "java"] {
                if let Some(value) = depends.get(field) {
                    let range = serde_json::to_string(value)?;
                    if range.len() <= 512 {
                        add_unique(
                            &mut collected.metadata.compatibility,
                            format!("{field}: {range}"),
                        );
                    }
                }
            }
        }
        if collected.metadata.preview_entry.is_none() {
            collected.metadata.preview_entry = fabric_icon(record.get("icon"));
        }
    }
    Ok(())
}

fn fabric_icon(value: Option<&Value>) -> Option<String> {
    let value = value?;
    let value = if let Some(value) = value.as_str() {
        value
    } else {
        value
            .as_object()?
            .iter()
            .filter_map(|(size, path)| {
                let size = size.parse::<u32>().ok()?;
                (size > 0 && size <= 1024).then_some((size.abs_diff(128), path.as_str()?))
            })
            .min_by_key(|(distance, _)| *distance)?
            .1
    };
    if value.len() <= 1024 {
        Some(value.to_string())
    } else {
        None
    }
}

fn toml_text(value: Option<&toml::Value>, limit: usize) -> Option<String> {
    value
        .and_then(toml::Value::as_str)
        .and_then(|value| bounded_text(value, limit))
}

fn parse_forge(bytes: &[u8], neo: bool, collected: &mut Collected) -> Result<()> {
    let root: toml::Value = toml::from_str(std::str::from_utf8(bytes)?)?;
    let records = root
        .get("mods")
        .and_then(toml::Value::as_array)
        .context("缺少 [[mods]] 数组")?;
    if records.is_empty() {
        bail!("[[mods]] 数组为空");
    }
    let loader = if neo { "NeoForge" } else { "Forge" };
    add_unique(&mut collected.metadata.loaders, loader.into());
    if let Some(value) = toml_text(root.get("loaderVersion"), 512) {
        add_unique(
            &mut collected.metadata.compatibility,
            format!("{loader} loaderVersion: {value}"),
        );
    }
    let multi = records.len() > 1;
    for record in records.iter().take(MAX_RECORDS) {
        let id = toml_text(record.get("modId"), 512);
        let version = toml_text(record.get("version"), 512);
        if version
            .as_deref()
            .is_some_and(|version| version.contains("${"))
        {
            collected.problem("模组版本含构建占位符，保留原声明且不猜测实际版本".into());
        }
        if id.is_none() {
            collected.problem(format!("{loader} 模组未声明 modId"));
        }
        collected.record(
            id,
            toml_text(record.get("displayName"), 512),
            version,
            toml_text(record.get("description"), 8192),
            multi,
        );
        if let Some(author) = toml_text(record.get("authors"), 1024) {
            add_unique(&mut collected.metadata.authors, author);
        }
        if collected.metadata.preview_entry.is_none() {
            collected.metadata.preview_entry = toml_text(record.get("logoFile"), 1024);
        }
    }
    if records.len() > MAX_RECORDS {
        collected.problem("单个 JAR 模组条目超过 256，已限制展示".into());
    }
    if let Some(dependencies) = root.get("dependencies").and_then(toml::Value::as_table) {
        for (owner, values) in dependencies.iter().take(MAX_RECORDS) {
            if let Some(values) = values.as_array() {
                for value in values.iter().take(MAX_RECORDS) {
                    let id = toml_text(value.get("modId"), 512);
                    if matches!(id.as_deref(), Some("minecraft" | "forge" | "neoforge"))
                        && let Some(range) = toml_text(value.get("versionRange"), 512)
                    {
                        let owner = bounded_text(owner, 512).unwrap_or_default();
                        add_unique(
                            &mut collected.metadata.compatibility,
                            format!("{owner} → {}: {range}", id.unwrap()),
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

fn parse_legacy_forge(bytes: &[u8], collected: &mut Collected) -> Result<()> {
    let root: Value = serde_json::from_slice(bytes)?;
    let entries = root.get("modList").unwrap_or(&root);
    let records = entries.as_array().context("mcmod.info 缺少模组数组")?;
    if records.is_empty() {
        bail!("mcmod.info 模组数组为空");
    }
    add_unique(
        &mut collected.metadata.loaders,
        "Forge（mcmod.info）".into(),
    );
    let multi = records.len() > 1;
    for record in records.iter().take(MAX_RECORDS) {
        collected.record(
            text(record.get("modid")),
            text(record.get("name")),
            text(record.get("version")),
            record
                .get("description")
                .and_then(Value::as_str)
                .and_then(|value| bounded_text(value, 8192)),
            multi,
        );
        if let Some(authors) = record.get("authorList").and_then(Value::as_array) {
            for author in authors.iter().take(MAX_RECORDS) {
                if let Some(author) = text(Some(author)) {
                    add_unique(&mut collected.metadata.authors, author);
                }
            }
        }
        if let Some(version) = text(record.get("mcversion")) {
            add_unique(
                &mut collected.metadata.compatibility,
                format!("minecraft: {version}"),
            );
        }
        if collected.metadata.preview_entry.is_none() {
            collected.metadata.preview_entry = text(record.get("logoFile"));
        }
    }
    Ok(())
}
