use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

/// 拒绝路径中的所有重解析点，包括目录联接和符号链接。
pub fn checked_path(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!("请选择绝对路径：{}", path.display());
    }
    let mut current = PathBuf::new();
    for component in path.components() {
        if matches!(component, Component::ParentDir) {
            bail!("路径不能含有 ..：{}", path.display());
        }
        current.push(component.as_os_str());
        if matches!(component, Component::Prefix(_) | Component::RootDir) {
            continue;
        }
        let metadata = fs::symlink_metadata(&current)
            .with_context(|| format!("路径不可访问：{}", current.display()))?;
        if is_link(&metadata) {
            bail!("默认不跟随符号链接或重解析点：{}", current.display());
        }
    }
    fs::canonicalize(path).with_context(|| format!("无法解析路径：{}", path.display()))
}

pub fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

pub fn path_key(path: &Path) -> String {
    let text = display_path(path);
    #[cfg(windows)]
    {
        text.to_lowercase().replace('/', "\\")
    }
    #[cfg(not(windows))]
    {
        text
    }
}

pub fn display_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    if let Some(unc) = text.strip_prefix("\\\\?\\UNC\\") {
        format!("\\\\{unc}")
    } else {
        text.strip_prefix("\\\\?\\").unwrap_or(&text).to_string()
    }
}

pub fn modified_ms(path: &Path) -> Option<i64> {
    fs::symlink_metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as i64)
}

pub fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    use std::io::Read;
    let checked = checked_path(path)?;
    let file = fs::File::open(checked)?;
    if file.metadata()?.len() > limit {
        bail!("文件超出元数据读取上限：{}", path.display());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        bail!("文件超出元数据读取上限");
    }
    Ok(bytes)
}

pub fn reveal(path: &Path) -> Result<()> {
    let path = checked_path(path)?;
    #[cfg(windows)]
    {
        use std::{os::windows::ffi::OsStrExt, ptr};
        use windows_sys::Win32::{
            System::Com::{
                COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize,
            },
            UI::Shell::{SHOpenFolderAndSelectItems, SHParseDisplayName},
        };
        // 用原生 PIDL 定位，避免资源管理器命令行的引号和逗号歧义。
        // COM 和 PIDL 的生命周期均留在当前后台线程。
        unsafe {
            let init = CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32);
            if init < 0 && init != 0x80010106u32 as i32 {
                bail!("Windows COM 初始化失败：0x{:08X}", init as u32);
            }
            let wide: Vec<u16> = std::ffi::OsStr::new(&display_path(&path))
                .encode_wide()
                .chain(Some(0))
                .collect();
            let mut pidl = ptr::null_mut();
            let parsed = SHParseDisplayName(
                wide.as_ptr(),
                ptr::null_mut(),
                &mut pidl,
                0,
                ptr::null_mut(),
            );
            let selected = if parsed >= 0 && !pidl.is_null() {
                SHOpenFolderAndSelectItems(pidl, 0, ptr::null(), 0)
            } else if parsed >= 0 {
                0x80004005u32 as i32
            } else {
                parsed
            };
            if !pidl.is_null() {
                CoTaskMemFree(pidl.cast());
            }
            if init >= 0 {
                CoUninitialize();
            }
            if selected < 0 {
                bail!(
                    "Windows 无法定位此路径（0x{:08X}）：{}",
                    selected as u32,
                    display_path(&path)
                );
            }
        }
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("xdg-open")
            .arg(path.parent().unwrap_or(&path))
            .spawn()?;
    }
    Ok(())
}
