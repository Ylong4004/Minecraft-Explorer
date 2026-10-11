use crate::platform::read_bounded;
use anyhow::{Result, bail};
use flate2::read::GzDecoder;
use serde::Deserialize;
use std::{io::Read, path::Path};

const NBT_LIMIT: u64 = 16 * 1024 * 1024;

#[derive(Default, Deserialize)]
struct Level {
    #[serde(rename = "Data")]
    data: Data,
}

#[derive(Default, Deserialize)]
struct Data {
    #[serde(rename = "LevelName")]
    level_name: Option<String>,
    #[serde(rename = "LastPlayed")]
    last_played: Option<i64>,
    #[serde(rename = "Version")]
    version: Option<Version>,
}

#[derive(Default, Deserialize)]
struct Version {
    #[serde(rename = "Name")]
    name: Option<String>,
}

#[derive(Default, Debug)]
pub struct WorldMetadata {
    pub name: Option<String>,
    pub last_played_ms: Option<i64>,
    pub game_version: Option<String>,
    pub status: String,
}

fn parse_level(path: &Path) -> Result<Data> {
    let raw = read_bounded(path, NBT_LIMIT)?;
    let bytes = if raw.starts_with(&[0x1f, 0x8b]) {
        let mut output = Vec::new();
        GzDecoder::new(raw.as_slice())
            .take(NBT_LIMIT + 1)
            .read_to_end(&mut output)?;
        if output.len() as u64 > NBT_LIMIT {
            bail!("NBT 解压后超过 16 MiB 上限");
        }
        output
    } else {
        raw
    };
    let level: Level = fastnbt::from_bytes(&bytes)?;
    Ok(level.data)
}

pub fn world_metadata(path: &Path) -> WorldMetadata {
    let mut failures = Vec::new();
    for file in ["level.dat", "level.dat_old"] {
        match parse_level(&path.join(file)) {
            Ok(data) => {
                let name = data.level_name.filter(|v| !v.trim().is_empty());
                let time = data
                    .last_played
                    .filter(|v| *v > 0 && *v <= crate::models::now_ms() + 86_400_000);
                return WorldMetadata {
                    status: format!(
                        "{file}{}{}",
                        if name.is_none() {
                            "；名称采用目录名"
                        } else {
                            ""
                        },
                        if time.is_none() {
                            "；时间采用文件系统修改时间"
                        } else {
                            ""
                        }
                    ),
                    name,
                    last_played_ms: time,
                    game_version: data.version.and_then(|v| v.name),
                };
            }
            Err(error) => failures.push(format!("{file}: {error}")),
        }
    }
    WorldMetadata {
        status: format!(
            "替代信息：目录名 / 文件系统修改时间；{}",
            failures.join("；")
        ),
        ..Default::default()
    }
}

#[derive(Debug, Default)]
pub struct VersionMetadata {
    pub id: Option<String>,
    pub inherits: Option<String>,
    pub loader: Option<String>,
    pub status: String,
}

pub fn version_metadata(path: &Path) -> VersionMetadata {
    match read_bounded(path, 4 * 1024 * 1024)
        .and_then(|bytes| Ok(serde_json::from_slice::<serde_json::Value>(&bytes)?))
    {
        Ok(json) => {
            let libraries: Vec<&str> = json
                .get("libraries")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .filter_map(|v| v.get("name").and_then(|v| v.as_str()))
                .collect();
            let loader = [
                ("net.fabricmc:fabric-loader:", "Fabric"),
                ("net.neoforged:neoforge:", "NeoForge"),
                ("net.minecraftforge:forge:", "Forge"),
                ("org.quiltmc:quilt-loader:", "Quilt"),
                ("optifine:OptiFine:", "OptiFine"),
            ]
            .into_iter()
            .find_map(|(prefix, name)| {
                libraries
                    .iter()
                    .any(|v| v.starts_with(prefix))
                    .then(|| name.to_string())
            });
            VersionMetadata {
                id: json.get("id").and_then(|v| v.as_str()).map(str::to_owned),
                inherits: json
                    .get("inheritsFrom")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned),
                loader,
                status: "版本 JSON；游戏目录关联由实际内容或用户指定确认".into(),
            }
        }
        Err(error) => VersionMetadata {
            status: format!("未识别版本定义：{error}"),
            ..Default::default()
        },
    }
}
