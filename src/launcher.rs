use crate::{models::*, platform::*};
use anyhow::{Context, Result, bail};
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capability {
    OpenLauncher,
    SelectInstance,
    LaunchInstance,
    TrackGameProcess,
}

pub trait LauncherAdapter {
    fn capabilities(&self) -> &[Capability];
    fn open_plan(&self, installation: &LauncherInstallation) -> Result<ProcessPlan>;
}

pub struct Pcl2Adapter;
pub struct PclCeAdapter;
pub struct HmclAdapter;

#[derive(Clone, Debug)]
pub struct ProcessPlan {
    pub program: PathBuf,
    pub args: Vec<std::ffi::OsString>,
    pub working_dir: PathBuf,
}

fn extension(path: &Path) -> String {
    path.extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase()
}

pub fn validate_installation(installation: &LauncherInstallation) -> Result<()> {
    let executable = checked_path(&installation.executable)?;
    if !executable.is_file() {
        bail!("启动器文件不存在");
    }
    if !checked_path(&installation.working_dir)?.is_dir() {
        bail!("启动器工作目录不可用");
    }
    if !checked_path(&installation.root)?.is_dir() {
        bail!("关联的 Minecraft 根目录不可用");
    }
    match (&installation.kind, extension(&executable).as_str()) {
        (LauncherKind::Pcl2, "exe")
        | (LauncherKind::PclCe, "exe")
        | (LauncherKind::Hmcl, "exe") => (),
        (LauncherKind::Hmcl, "jar") => {
            let java = installation
                .java
                .as_ref()
                .context("HMCL JAR 需要指定启动 HMCL 使用的 java.exe 或 javaw.exe")?;
            if !checked_path(java)?.is_file() {
                bail!("指定的 Java 文件不可用");
            }
            let name = java
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            if !["java.exe", "javaw.exe", "java", "javaw"].contains(&name.as_str()) {
                bail!("请选择 Java 可执行文件");
            }
        }
        _ => bail!("PCL2 / PCL CE 只接受 EXE；HMCL 接受 EXE 或 JAR"),
    }
    Ok(())
}

fn native_open_plan(
    installation: &LauncherInstallation,
    expected_kind: LauncherKind,
) -> Result<ProcessPlan> {
    if installation.kind != expected_kind {
        bail!("启动器安装类型与 {} 适配器不一致", expected_kind.label());
    }
    validate_installation(installation)?;
    Ok(ProcessPlan {
        program: installation.executable.clone(),
        args: vec![],
        working_dir: installation.working_dir.clone(),
    })
}

impl LauncherAdapter for Pcl2Adapter {
    fn capabilities(&self) -> &[Capability] {
        &[Capability::OpenLauncher]
    }
    fn open_plan(&self, installation: &LauncherInstallation) -> Result<ProcessPlan> {
        native_open_plan(installation, LauncherKind::Pcl2)
    }
}

impl LauncherAdapter for PclCeAdapter {
    fn capabilities(&self) -> &[Capability] {
        &[Capability::OpenLauncher]
    }
    fn open_plan(&self, installation: &LauncherInstallation) -> Result<ProcessPlan> {
        native_open_plan(installation, LauncherKind::PclCe)
    }
}

impl LauncherAdapter for HmclAdapter {
    fn capabilities(&self) -> &[Capability] {
        &[Capability::OpenLauncher]
    }
    fn open_plan(&self, installation: &LauncherInstallation) -> Result<ProcessPlan> {
        if installation.kind != LauncherKind::Hmcl {
            bail!("启动器安装类型与 HMCL 适配器不一致");
        }
        validate_installation(installation)?;
        let jar = extension(&installation.executable) == "jar";
        Ok(ProcessPlan {
            program: if jar {
                installation.java.clone().context("未设置 Java")?
            } else {
                installation.executable.clone()
            },
            args: if jar {
                vec![
                    "-jar".into(),
                    installation.executable.clone().into_os_string(),
                ]
            } else {
                vec![]
            },
            working_dir: installation.working_dir.clone(),
        })
    }
}

pub fn adapter(kind: &LauncherKind) -> Box<dyn LauncherAdapter> {
    match kind {
        LauncherKind::Pcl2 => Box::new(Pcl2Adapter),
        LauncherKind::PclCe => Box::new(PclCeAdapter),
        LauncherKind::Hmcl => Box::new(HmclAdapter),
    }
}

pub enum LauncherProcess {
    Direct(Child),
    #[cfg(windows)]
    Shell(std::os::windows::io::OwnedHandle),
}

impl LauncherProcess {
    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        match self {
            Self::Direct(child) => child.try_wait(),
            #[cfg(windows)]
            Self::Shell(handle) => {
                use std::os::windows::{io::AsRawHandle, process::ExitStatusExt};
                use windows_sys::Win32::System::Threading::{
                    GetExitCodeProcess, WaitForSingleObject,
                };
                let handle = handle.as_raw_handle();
                match unsafe { WaitForSingleObject(handle, 0) } {
                    258 => Ok(None),
                    0 => {
                        let mut code = 0;
                        if unsafe { GetExitCodeProcess(handle, &mut code) } == 0 {
                            return Err(std::io::Error::last_os_error());
                        }
                        Ok(Some(ExitStatus::from_raw(code)))
                    }
                    _ => Err(std::io::Error::last_os_error()),
                }
            }
        }
    }
    pub fn wait(&mut self) -> std::io::Result<ExitStatus> {
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}

#[cfg(windows)]
fn spawn_elevated(plan: &ProcessPlan) -> Result<LauncherProcess> {
    if !plan.args.is_empty() || extension(&plan.program) != "exe" {
        bail!("启动程序需要管理员权限，请检查 Java 或启动器的兼容性设置");
    }
    shell_open(plan)
}

#[cfg(windows)]
fn shell_open(plan: &ProcessPlan) -> Result<LauncherProcess> {
    use std::{
        os::windows::{ffi::OsStrExt, io::FromRawHandle},
        ptr,
    };
    use windows_sys::Win32::{
        System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
        UI::Shell::{
            SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
            ShellExecuteExW,
        },
    };
    // 由系统处理明确选择的文件，不拼接批处理命令或读取账号参数。
    let wide = |value: &str| {
        std::ffi::OsStr::new(value)
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    let program = wide(&display_path(&plan.program));
    let directory = wide(&display_path(&plan.working_dir));
    let verb = wide("open");
    let initialized = unsafe { CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32) };
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
    info.lpVerb = verb.as_ptr();
    info.lpFile = program.as_ptr();
    info.lpDirectory = directory.as_ptr();
    info.nShow = if matches!(extension(&plan.program).as_str(), "bat" | "cmd") {
        0
    } else {
        1
    };
    let success = unsafe { ShellExecuteExW(&mut info) };
    let error = if success == 0 {
        Some(std::io::Error::last_os_error())
    } else {
        None
    };
    if initialized >= 0 {
        unsafe { CoUninitialize() };
    }
    if let Some(error) = error {
        if error.raw_os_error() == Some(1223) {
            bail!("已取消 Windows 管理员授权，启动器未打开");
        }
        return Err(error).context("Windows 无法打开需要管理员权限的启动器");
    }
    if info.hProcess.is_null() {
        bail!("Windows 没有返回启动器进程句柄，请检查已打开的启动器窗口");
    }
    Ok(LauncherProcess::Shell(unsafe {
        std::os::windows::io::OwnedHandle::from_raw_handle(info.hProcess)
    }))
}

pub fn spawn(plan: &ProcessPlan) -> Result<LauncherProcess> {
    let mut command = Command::new(&plan.program);
    command.args(&plan.args).current_dir(&plan.working_dir);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // 隐藏 Java 包装程序的控制台窗口。
    }
    match command.spawn() {
        Ok(child) => Ok(LauncherProcess::Direct(child)),
        #[cfg(windows)]
        Err(error) if error.raw_os_error() == Some(740) => spawn_elevated(plan),
        Err(error) => {
            Err(error).with_context(|| format!("无法打开启动器：{}", display_path(&plan.program)))
        }
    }
}

pub fn validate_mapping(
    mapping: &LaunchMapping,
    installation: &LauncherInstallation,
    version: &Item,
) -> Result<()> {
    if mapping.launcher_id != installation.id {
        bail!("映射与启动器安装 ID 不一致");
    }
    if path_key(&mapping.root) != path_key(&installation.root)
        || path_key(&version.root) != path_key(&mapping.root)
    {
        bail!("启动器、映射和版本属于不同的 Minecraft 根目录");
    }
    if version.kind != ItemKind::Version
        || version.version_id.as_deref() != Some(mapping.version_id.as_str())
        || version.path != mapping.version_path
    {
        bail!("映射中的实例 ID 或真实版本路径已经失效");
    }
    checked_path(&mapping.game_dir)?;
    checked_path(&version.path)?;
    if let Some(actual) = &version.game_dir
        && path_key(actual) != path_key(&mapping.game_dir)
    {
        bail!("版本的实际独立游戏目录已改变，原映射失效");
    }
    Ok(())
}

pub fn create_mapping(
    subject: &Item,
    installation: &LauncherInstallation,
    version: &Item,
    game_dir: &Path,
) -> Result<LaunchMapping> {
    let game_dir = checked_path(game_dir)?;
    if !game_dir.is_dir() {
        bail!("游戏目录不可用");
    }
    if let Some(actual) = &subject.game_dir
        && path_key(actual) != path_key(&game_dir)
    {
        bail!("所选条目的实际游戏目录不能更改为另一个目录");
    }
    if path_key(&subject.root) != path_key(&version.root) {
        bail!("所选条目与目标版本来自不同根目录");
    }
    if let Some(independent) = &version.game_dir
        && path_key(independent) != path_key(&game_dir)
    {
        bail!("目标版本的独立游戏目录与所选条目不同");
    }
    let mapping = LaunchMapping {
        subject_id: subject.id.clone(),
        launcher_id: installation.id.clone(),
        root: version.root.clone(),
        game_dir,
        version_id: version
            .version_id
            .clone()
            .context("版本 JSON 缺少 id，不能猜测")?,
        version_path: version.path.clone(),
        source: "用户手动关联".into(),
        script: None,
    };
    validate_mapping(&mapping, installation, version)?;
    Ok(mapping)
}

pub fn validate_script(script: &LaunchScript) -> Result<()> {
    let path = checked_path(&script.path)?;
    if !path.is_file() || !matches!(extension(&path).as_str(), "bat" | "cmd") {
        bail!("请选择该版本导出的 BAT 或 CMD 启动脚本");
    }
    if !checked_path(&script.working_dir)?.is_dir() {
        bail!("启动脚本工作目录不可用");
    }
    Ok(())
}

/// 脚本是用户明确关联的启动入口；不代表启动器提供指定实例 CLI。
pub fn spawn_script(
    installation: &LauncherInstallation,
    mapping: &LaunchMapping,
    version: &Item,
) -> Result<LauncherProcess> {
    validate_mapping(mapping, installation, version)?;
    let script = mapping
        .script
        .as_ref()
        .context("请先在设置关联中绑定该版本的启动脚本")?;
    validate_script(script)?;
    #[cfg(windows)]
    {
        shell_open(&ProcessPlan {
            program: script.path.clone(),
            args: vec![],
            working_dir: script.working_dir.clone(),
        })
    }
    #[cfg(not(windows))]
    {
        bail!("BAT / CMD 启动脚本仅支持 Windows")
    }
}

/// 保留直接启动门禁：外部实例启动接口尚未通过真实验收。
pub fn launch_instance(
    installation: &LauncherInstallation,
    mapping: &LaunchMapping,
    version: &Item,
) -> Result<()> {
    validate_mapping(mapping, installation, version)?;
    if !adapter(&installation.kind)
        .capabilities()
        .contains(&Capability::LaunchInstance)
    {
        bail!(
            "OpenOnly：尚未验证外部指定实例启动接口。请打开启动器后手动选择 {}（{}）。也可手动选择关联同一根目录的其他启动器安装。",
            mapping.version_id,
            display_path(&mapping.root)
        );
    }
    bail!("当前适配器没有已验证的直接启动实现")
}
