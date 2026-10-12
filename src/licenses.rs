use anyhow::{Context, Result, ensure};
use flate2::read::GzDecoder;
use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};

/// 许可随单文件 EXE 嵌入，点击关于页按钮时才导出到本程序数据目录。
pub fn export(directory: &Path) -> Result<PathBuf> {
    let directory = crate::platform::checked_path(directory)?;
    let path = directory.join(format!(
        "MinecraftExplorer-{}-licenses.txt",
        env!("CARGO_PKG_VERSION")
    ));
    let bytes = texts()?;
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            file.write_all(&bytes).context("写入许可文件失败")?;
            file.sync_all()?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = crate::platform::checked_path(&path)?;
            ensure!(
                crate::platform::read_bounded(&existing, 16 * 1024 * 1024)? == bytes,
                "已有许可文件内容不同，请先手动移动该文件后重试：{}",
                crate::platform::display_path(&path)
            );
        }
        Err(error) => return Err(error).context("创建许可文件失败"),
    }
    Ok(path)
}

pub fn texts() -> Result<Vec<u8>> {
    use std::io::Read;
    let mut result = Vec::new();
    GzDecoder::new(&include_bytes!("../assets/third-party-licenses.txt.gz")[..])
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut result)
        .context("内置许可解压失败")?;
    ensure!(result.len() <= 16 * 1024 * 1024, "内置许可超出大小上限");
    Ok(result)
}
