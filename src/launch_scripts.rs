use crate::{models::*, platform::*};
use anyhow::{Context, Result, ensure};
use regex::Regex;
use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::LazyLock,
};

static JAVA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)^\s*@?(?:"[^"\r\n]+[\\/]javaw?\.exe"|[^\s"]+[\\/]javaw?\.exe)\s"#).unwrap()
});
static VERSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:^|\s)--version\s+(?:"([^"\r\n]+)"|([^\s"]+))"#).unwrap());
static GAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:^|\s)--gameDir\s+(?:"([^"\r\n]+)"|([^\s"]+))"#).unwrap());

/// 仅读取已注册 PCL 安装的固定脚本位置，不扫描账号文件，也不解析登录参数。
pub fn discover(
    installation: &LauncherInstallation,
    mapping: &LaunchMapping,
    data: &Path,
) -> Result<Option<LaunchScript>> {
    ensure!(
        installation.id == mapping.launcher_id
            && path_key(&installation.root) == path_key(&mapping.root),
        "脚本关联与所选启动器或根目录不一致"
    );
    if !matches!(installation.kind, LauncherKind::Pcl2 | LauncherKind::PclCe) {
        return Ok(None);
    }
    let work = checked_path(&installation.working_dir)?;
    let mut locations = vec![work.join("PCL/LatestLaunch.bat")];
    if let Some(parent) = installation.executable.parent() {
        locations.push(parent.join("PCL/LatestLaunch.bat"));
    }
    let mut seen = std::collections::HashSet::new();
    for path in locations {
        if !seen.insert(path_key(&path)) {
            continue;
        }
        let Ok(bytes) = read_bounded(&path, 1024 * 1024) else {
            continue;
        };
        if !matches_target(&bytes, mapping) {
            continue;
        }
        // 上次启动脚本会被覆盖，保存经过校验的字节副本，避免关联随后指向其他版本。
        let directory = checked_path(data)?.join("launch-scripts");
        std::fs::create_dir_all(&directory)?;
        let directory = checked_path(&directory)?;
        let target = directory.join(format!("{}.bat", blake3::hash(&bytes).to_hex()));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
        {
            Ok(mut output) => {
                output.write_all(&bytes)?;
                output.sync_all()?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                ensure!(
                    read_bounded(&target, 1024 * 1024)? == bytes,
                    "已有自动启动脚本副本内容不同，请手动检查程序数据目录"
                );
            }
            Err(error) => return Err(error).context("无法保存自动识别的启动脚本副本"),
        }
        return Ok(Some(LaunchScript {
            path: target,
            working_dir: work,
        }));
    }
    Ok(None)
}

fn matches_target(bytes: &[u8], mapping: &LaunchMapping) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let lines: Vec<_> = text
        .trim_start_matches('\u{feff}')
        .lines()
        .filter(|line| JAVA.is_match(line))
        .collect();
    if lines.len() != 1 {
        return false;
    }
    let literal = |pattern: &Regex| -> Option<String> {
        let mut captures = pattern.captures_iter(lines[0]);
        let found = captures.next()?;
        if captures.next().is_some() {
            return None;
        }
        let value = found.get(1).or_else(|| found.get(2))?.as_str();
        // 不执行或展开批处理变量、延迟展开和转义；无法确定的格式留给手动关联。
        if value.contains(['%', '!', '^']) {
            return None;
        }
        Some(value.to_string())
    };
    let Some(version) = literal(&VERSION) else {
        return false;
    };
    let Some(game) = literal(&GAME) else {
        return false;
    };
    if version != mapping.version_id {
        return false;
    }
    let game = PathBuf::from(game);
    game.is_absolute()
        && checked_path(&game).is_ok_and(|path| path_key(&path) == path_key(&mapping.game_dir))
}
