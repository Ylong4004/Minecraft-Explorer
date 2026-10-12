use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum SourceKind {
    Root,
    Custom,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Source {
    pub path: PathBuf,
    pub kind: SourceKind,
    #[serde(default)]
    pub associated_root: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ItemKind {
    World,
    Version,
    Instance,
    ResourcePack,
    Mod,
    ShaderPack,
    DataPack,
}

impl ItemKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::World => "world",
            Self::Version => "version",
            Self::Instance => "instance",
            Self::ResourcePack => "resource_pack",
            Self::Mod => "mod",
            Self::ShaderPack => "shader_pack",
            Self::DataPack => "data_pack",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::World => "存档",
            Self::Version => "版本定义",
            Self::Instance => "游戏目录",
            Self::ResourcePack => "资源包",
            Self::Mod => "模组",
            Self::ShaderPack => "光影包",
            Self::DataPack => "数据包",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ContentMetadata {
    pub description: Option<String>,
    pub declared_id: Option<String>,
    pub declared_version: Option<String>,
    pub authors: Vec<String>,
    pub loaders: Vec<String>,
    pub compatibility: Vec<String>,
    pub preview_entry: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub kind: ItemKind,
    pub source: String,
    pub root: PathBuf,
    pub path: PathBuf,
    pub name: String,
    pub folder: String,
    pub game_dir: Option<PathBuf>,
    pub ownership: String,
    pub version_id: Option<String>,
    pub game_version: Option<String>,
    pub loader: Option<String>,
    pub size: Option<u64>,
    pub size_complete: bool,
    pub modified_ms: Option<i64>,
    pub last_played_ms: Option<i64>,
    pub metadata_status: String,
    pub icon: Option<PathBuf>,
    #[serde(default)]
    pub preview_modified_ms: Option<i64>,
    pub available: bool,
    #[serde(default)]
    pub content: Option<ContentMetadata>,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub last_opened_ms: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum LauncherKind {
    Pcl2,
    Hmcl,
    PclCe,
}

impl LauncherKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Pcl2 => "PCL2",
            Self::Hmcl => "HMCL",
            Self::PclCe => "PCL CE",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LauncherInstallation {
    pub id: String,
    pub kind: LauncherKind,
    pub executable: PathBuf,
    pub working_dir: PathBuf,
    pub root: PathBuf,
    pub java: Option<PathBuf>,
    pub checked_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaunchScript {
    pub path: PathBuf,
    pub working_dir: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaunchMapping {
    pub subject_id: String,
    pub launcher_id: String,
    pub root: PathBuf,
    pub game_dir: PathBuf,
    pub version_id: String,
    pub version_path: PathBuf,
    pub source: String,
    #[serde(default)]
    pub script: Option<LaunchScript>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    pub sources: Vec<Source>,
    pub launchers: Vec<LauncherInstallation>,
    pub mappings: Vec<LaunchMapping>,
    pub default_launcher: Option<String>,
    #[serde(default)]
    pub instance_defaults: std::collections::HashMap<String, String>,
    #[serde(default = "default_watch")]
    pub watch_enabled: bool,
    #[serde(default)]
    pub english: bool,
    #[serde(default)]
    pub light_theme: bool,
}

fn default_watch() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            sources: Vec::new(),
            launchers: Vec::new(),
            mappings: Vec::new(),
            default_launcher: None,
            instance_defaults: Default::default(),
            watch_enabled: true,
            english: false,
            light_theme: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Name,
    SizeDesc,
    SizeAsc,
    TimeDesc,
    TimeAsc,
    Ownership,
    Recent,
}

#[derive(Clone, Debug, Default)]
pub struct Query {
    pub text: String,
    pub kind: Option<ItemKind>,
    pub game_dir: Option<String>,
    pub source: Option<String>,
    pub sort: Sort,
    pub favorite_only: bool,
    pub tag: Option<String>,
    pub recent_only: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub worlds: u64,
    pub versions: u64,
    pub instances: u64,
    pub world_bytes: u64,
    pub resource_packs: u64,
    pub mods: u64,
    pub shader_packs: u64,
    pub data_packs: u64,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub fn bytes_label(value: Option<u64>, complete: bool) -> String {
    let Some(bytes) = value else {
        return "待统计".into();
    };
    let units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut number = bytes as f64;
    let mut unit = 0;
    while number >= 1024.0 && unit < units.len() - 1 {
        number /= 1024.0;
        unit += 1;
    }
    format!(
        "{}{number:.1} {}",
        if complete { "" } else { "≥ " },
        units[unit]
    )
}
