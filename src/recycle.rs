//! 通过 Windows Shell 仅移入回收站；进度回调在触碰文件前
//! 否决任何永久删除操作。
use anyhow::{Result, bail};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

pub fn recycle_flags_allowed(flags: u32) -> bool {
    flags & 0x80 != 0
}

#[cfg(not(windows))]
pub fn send(_path: &Path, _cancel: &AtomicBool) -> Result<()> {
    bail!("仅 Windows 提供已验证的回收站操作；未执行删除")
}

#[cfg(windows)]
pub fn send(path: &Path, cancel: &AtomicBool) -> Result<()> {
    use crate::platform::{checked_path, display_path};
    use std::{os::windows::ffi::OsStrExt, sync::Arc};
    use windows::{
        Win32::{
            System::Com::{
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoUninitialize,
            },
            UI::Shell::{
                FOF_NO_CONNECTED_ELEMENTS, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT,
                FOFX_ADDUNDORECORD, FOFX_EARLYFAILURE, FOFX_RECYCLEONDELETE, FileOperation,
                IFileOperation, IFileOperationProgressSink, IShellItem,
                SHCreateItemFromParsingName,
            },
        },
        core::PCWSTR,
    };
    if cancel.load(Ordering::SeqCst) {
        bail!("操作已取消，未删除")
    }
    let path = checked_path(path)?;
    let display = display_path(&path);
    // 不支持网络或可移动卷的回收站操作。
    let bytes = display.as_bytes();
    if bytes.len() < 3 || bytes[1] != b':' || bytes[2] != b'\\' {
        bail!("此路径不支持本程序的回收站操作，未删除")
    }
    let root: Vec<u16> = display[..3].encode_utf16().chain(Some(0)).collect();
    if unsafe { windows_sys::Win32::Storage::FileSystem::GetDriveTypeW(root.as_ptr()) } != 3 {
        bail!("回收站仅开放本机固定磁盘；网络/移动磁盘未删除")
    }
    let wide: Vec<u16> = std::ffi::OsStr::new(&display)
        .encode_wide()
        .chain(Some(0))
        .collect();
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        struct Apartment;
        impl Drop for Apartment {
            fn drop(&mut self) {
                unsafe {
                    CoUninitialize();
                }
            }
        }
        let _apartment = Apartment;
        let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None)?;
        let operation: IFileOperation =
            CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER)?;
        operation.SetOperationFlags(
            FOFX_RECYCLEONDELETE
                | FOFX_EARLYFAILURE
                | FOFX_ADDUNDORECORD
                | FOF_NOERRORUI
                | FOF_SILENT
                | FOF_NOCONFIRMATION
                | FOF_NO_CONNECTED_ELEMENTS,
        )?;
        let successful = Arc::new(AtomicBool::new(false));
        let refused = Arc::new(AtomicBool::new(false));
        let sink: IFileOperationProgressSink = RecycleSink {
            successful: successful.clone(),
            refused: refused.clone(),
        }
        .into();
        operation.DeleteItem(&item, &sink)?;
        let performed = operation.PerformOperations();
        if refused.load(Ordering::SeqCst) {
            bail!("Windows 无法确认移入回收站，已拒绝永久删除；原件保留")
        }
        performed?;
        if operation.GetAnyOperationsAborted()?.as_bool() || !successful.load(Ordering::SeqCst) {
            bail!("回收站操作未完成；请检查原路径，未执行永久删除")
        }
    }
    if path.exists() {
        bail!("回收站操作后源路径仍存在，请刷新检查")
    }
    Ok(())
}

#[cfg(windows)]
use windows::{
    Win32::UI::Shell::{IFileOperationProgressSink, IFileOperationProgressSink_Impl, IShellItem},
    core::{HRESULT, PCWSTR, Ref, implement},
};

#[cfg(windows)]
#[implement(IFileOperationProgressSink)]
struct RecycleSink {
    successful: std::sync::Arc<AtomicBool>,
    refused: std::sync::Arc<AtomicBool>,
}

#[cfg(windows)]
#[allow(non_snake_case)]
impl IFileOperationProgressSink_Impl for RecycleSink_Impl {
    fn StartOperations(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn FinishOperations(&self, result: HRESULT) -> windows::core::Result<()> {
        result.ok()
    }
    fn PreDeleteItem(&self, flags: u32, _item: Ref<IShellItem>) -> windows::core::Result<()> {
        if !recycle_flags_allowed(flags) {
            self.refused.store(true, Ordering::SeqCst);
            return Err(windows::core::Error::from_hresult(
                windows::Win32::Foundation::E_ABORT,
            ));
        }
        Ok(())
    }
    fn PostDeleteItem(
        &self,
        flags: u32,
        _item: Ref<IShellItem>,
        result: HRESULT,
        _new: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        if result.is_ok() && recycle_flags_allowed(flags) {
            self.successful.store(true, Ordering::SeqCst);
        }
        result.ok()
    }
    fn PreRenameItem(
        &self,
        _flags: u32,
        _item: Ref<IShellItem>,
        _name: &PCWSTR,
    ) -> windows::core::Result<()> {
        Err(windows::core::Error::from_hresult(
            windows::Win32::Foundation::E_ABORT,
        ))
    }
    fn PostRenameItem(
        &self,
        _flags: u32,
        _item: Ref<IShellItem>,
        _name: &PCWSTR,
        _result: HRESULT,
        _new: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PreMoveItem(
        &self,
        _flags: u32,
        _item: Ref<IShellItem>,
        _destination: Ref<IShellItem>,
        _name: &PCWSTR,
    ) -> windows::core::Result<()> {
        Err(windows::core::Error::from_hresult(
            windows::Win32::Foundation::E_ABORT,
        ))
    }
    fn PostMoveItem(
        &self,
        _flags: u32,
        _item: Ref<IShellItem>,
        _destination: Ref<IShellItem>,
        _name: &PCWSTR,
        _result: HRESULT,
        _new: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PreCopyItem(
        &self,
        _flags: u32,
        _item: Ref<IShellItem>,
        _destination: Ref<IShellItem>,
        _name: &PCWSTR,
    ) -> windows::core::Result<()> {
        Err(windows::core::Error::from_hresult(
            windows::Win32::Foundation::E_ABORT,
        ))
    }
    fn PostCopyItem(
        &self,
        _flags: u32,
        _item: Ref<IShellItem>,
        _destination: Ref<IShellItem>,
        _name: &PCWSTR,
        _result: HRESULT,
        _new: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PreNewItem(
        &self,
        _flags: u32,
        _destination: Ref<IShellItem>,
        _name: &PCWSTR,
    ) -> windows::core::Result<()> {
        Err(windows::core::Error::from_hresult(
            windows::Win32::Foundation::E_ABORT,
        ))
    }
    fn PostNewItem(
        &self,
        _flags: u32,
        _destination: Ref<IShellItem>,
        _name: &PCWSTR,
        _template: &PCWSTR,
        _attributes: u32,
        _result: HRESULT,
        _new: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn UpdateProgress(&self, _total: u32, _done: u32) -> windows::core::Result<()> {
        Ok(())
    }
    fn ResetTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn PauseTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn ResumeTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
}
