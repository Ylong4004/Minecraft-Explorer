# 启动器接口核查

## 0.4.1 再核查与用户绑定脚本（2026-10-12）

再次读取当前 [PCL 启动入口](https://github.com/Meloong-Git/PCL/blob/main/Plain%20Craft%20Launcher%202/Application.xaml.vb)、[PCL CE StartupService](https://github.com/PCL-Community/PCL-CE/blob/dev/PCL.Core/App/Essentials/StartupService.cs) 与 [HMCL UpdateHandler](https://github.com/HMCL-dev/HMCL/blob/main/HMCL/src/main/java/org/jackhuang/hmcl/upgrade/UpdateHandler.java)，仍未确认可以指定根目录及版本、并实测启动游戏的外部入口；原生适配器能力保持 OpenLauncher。

0.4.1 新增用户为具体映射绑定 BAT / CMD 的独立入口。[PCL 官方自定义事件文档](https://github.com/Meloong-Git/PCL/wiki/自定义事件) 描述 `LatestLaunch.bat` 保存上次启动脚本；因此不会自动发现或执行它，以免启动别的版本。用户需另存目标版本专用脚本，确认版本和原工作目录后绑定。脚本可能含有登录信息，本程序不读取其内容，不生成、不替换、不管理登录参数。

生成的无害 CMD 已通过 Windows 实际执行测试，路径含中文、空格、& 和括号，覆盖指定工作目录、映射持久化、旧配置兼容及跨根目录拒绝。没有用用户真实脚本启动 Minecraft，此测试不代替真实游戏和登录有效性验收。教程见根 README；下文保留此前核查记录。

## 阶段 4 实际打开验收（2026-10-12）

用户补充 PCL CE 绑定后无法打开，并明确要求再次打开。本机 EXE 的 Windows 兼容性设置为 `~ RUNASADMIN`，同路径 CreateProcess 实测返回错误 740；隔离副本无该路径兼容性设置，因此此前假启动器或副本测试未覆盖该故障。

通过系统 Shell 打开 `D:\PCL2 CE\PCL2_CE_Release_x64.exe` 后，实际窗口标题为 Plain Craft Launcher Community Edition，响应正常。修复后的 Rust `launcher::spawn` 也实测走到 Shell 提权分支，并取得仍在运行的进程句柄。使用原工作目录、不传游戏参数，未修改兼容性或启动器配置，未启动 Minecraft；启动器自身会正常生成日志/运行数据。

实现参考 [Microsoft ShellExecuteExW](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shellexecuteexw)。仅在系统明确返回 740 时处理管理员启动，无参数 EXE 使用 open 动词和独立工作目录，不拼接命令行。UAC 取消返回明确提示，不自动换其他安装。此验收确认 OpenLauncher，仍不能证明选择实例或 LaunchInstance。

下文阶段 2 的未运行说明是当时的历史记录；PCL2/HMCL 与指定实例直接启动的实测限制仍适用。

核查日期：2026-10-11。**未完成真实指定实例启动验收，当前适配器只启用 OpenLauncher。** 下述源码检查不是对所有发行版本“不支持接口”的证明；目前没有找到并实测通过可公开依赖的指定实例外部启动入口。

## PCL2

本机只读检查发现：`D:\PCL2\Plain Craft Launcher 2.exe`，FileVersion / ProductVersion **2.12.7.3**。未运行该真实安装、未改其设置。

[官方自定义事件文档](https://github.com/Meloong-Git/PCL/wiki/自定义事件) 中“启动游戏”是启动器内部事件，不可直接当作第三方进程 API。

检查 PCL 源码快照 `0e0d12fdce6a2804916fb2be60e41144da637c18` 的 [Application.xaml.vb](https://github.com/Meloong-Git/PCL/blob/0e0d12fdce6a2804916fb2be60e41144da637c18/Plain%20Craft%20Launcher%202/Application.xaml.vb)：启动入口参数分支处理更新、GPU、内存、等待等功能。所查入口未提供经验证的具体版本启动分支。该源码快照不能代替本机 2.12.7.3 二进制版本实测。

## PCL CE

PCL CE 使用独立的 `LauncherKind::PclCe` 和 `PclCeAdapter`；设置中的类型显示为 **PCL CE**。旧 `Pcl2` / `Hmcl` 配置值保持兼容。每个安装拥有独立 ID、EXE、工作目录和关联 Minecraft 根目录，映射与实例默认启动器使用安装 ID，不以程序文件名区分安装。

本机只读取得 `D:\PCL2 CE\PCL2_CE_Release_x64.exe` 的版本资源：

| 字段 | 值 |
| --- | --- |
| FileVersion | `2.14.5.0` |
| ProductVersion | `1.0.0+a5e79c7b7856c77e393da8c285dbbca33bfdafd1` |
| ProductName | `Plain Craft Launcher Community Edition` |
| 文件大小 | `20,741,041` 字节 |

当前官方仓库为 [PCL-Community/PCL-CE](https://github.com/PCL-Community/PCL-CE)。核查了与本机 ProductVersion 对应的源码提交 `a5e79c7b7856c77e393da8c285dbbca33bfdafd1`，以及截至核查日期的 `dev` 提交 `f943218eabd1f34651432031884a0606ecc5c6e2`：

- 本机对应提交的 [StartupService.cs](https://github.com/PCL-Community/PCL-CE/blob/a5e79c7b7856c77e393da8c285dbbca33bfdafd1/PCL.Core/App/Essentials/StartupService.cs) 注册 `update`、`activate`、`memory`、`promote` 子命令；当前提交的 [StartupService.cs](https://github.com/PCL-Community/PCL-CE/blob/f943218eabd1f34651432031884a0606ecc5c6e2/PCL.Core/App/Essentials/StartupService.cs) 注册更新、激活和提权子命令。所查注册表没有游戏实例启动命令。
- 本机对应提交的 [Application.xaml.vb](https://github.com/PCL-Community/PCL-CE/blob/a5e79c7b7856c77e393da8c285dbbca33bfdafd1/Plain%20Craft%20Launcher%202/Application.xaml.vb) 及当前的 [Application.xaml.cs](https://github.com/PCL-Community/PCL-CE/blob/f943218eabd1f34651432031884a0606ecc5c6e2/Plain%20Craft%20Launcher%202/Application.xaml.cs) 仍处理 `--gpu` 分支，不能将其当作指定 Minecraft 版本的启动协议。
- [RpcService.cs](https://github.com/PCL-Community/PCL-CE/blob/a5e79c7b7856c77e393da8c285dbbca33bfdafd1/PCL.Core/App/Essentials/RpcService.cs) 有命名管道 RPC 和 `ping` / `activate` 功能；`StartupService` 的 `cli` RPC 把命令交给已注册回调处理。RPC 能接收 JSON 或返回接收成功，不代表它选择了正确实例或启动了游戏。

以上源码与本机版本资源对应，是接口核查证据；没有运行真实 EXE，因此没有二进制打开、选择实例或游戏启动验收结果。只读核查不读取账号资料、登录令牌或启动器用户配置，也不发送 RPC 请求。

已实现的能力为 **OpenLauncher**：仅接受存在的 EXE，验证工作目录和关联根目录后，用独立进程参数数组打开用户所选安装，不传任何猜测的版本参数。其他能力继续关闭。启动失败或指定实例接口不可用时，由用户手动选择关联同一根目录的其他安装；不会自动运行另一默认实例。

## HMCL

阅读 [HMCL 主仓库源码快照](https://github.com/HMCL-dev/HMCL/tree/a6e74e7c19d59b54f2e1c86b0e71ab3f3ac61944) 的 Main、EntryPoint、Launcher 和 UpdateHandler。Launcher 将参数交给更新处理后启动 JavaFX；[UpdateHandler.processArguments](https://github.com/HMCL-dev/HMCL/blob/a6e74e7c19d59b54f2e1c86b0e71ab3f3ac61944/HMCL/src/main/java/org/jackhuang/hmcl/upgrade/UpdateHandler.java) 所查外部参数分支为更新迁移/`--apply-to`，不是实例启动入口。

[JVM 参数与环境变量文档](https://docs.hmcl.net/launcher/jvm-options-and-env.html) 描述运行 HMCL 的 JVM 设置，不能证明可以从外部指定实例启动 Minecraft。当前实现遵循 EXE 或 `Java -jar <HMCL.jar>` 的打开流程；本机已发现 Java 17，但未找到可用于真实版本验收的 HMCL 安装。Java 17 并不是对所有 HMCL 历史版本的统一要求，必须按用户实际发行确认。

## 实测范围与继续条件

自动化测试编译临时 Windows EXE，实际执行 Adapter 产生的 ProcessPlan，验证路径中的中文、空格、`&`、独立参数、工作目录，以及 HMCL JAR 的 `-jar` 参数。PCL CE 回归用两个同文件名的临时 EXE 验证实际所选安装和工作目录，覆盖 EXE/JAR 类型限制、混合启动器配置持久化、实例默认选择、安装 ID 和同名版本跨根目录拦截。测试使用模拟 Java 入口确认参数，并没有将其写成“HMCL / PCL CE 真实安装实测成功”。UI 禁用直接启动并显示能力限制。

后续按真实安装版本取得受支持的 CLI / IPC / URL scheme 说明；先验证 OpenLauncher，再验证选择实例、根目录、同名实例及真实 LaunchInstance。无明确接口时继续提供目标 ID、复制 ID、打开原启动器的流程。真实安装运行可能产生启动器正常日志/配置，执行前需明确本次测试范围；不会自行改配置、读登录令牌或让未知命令启动默认游戏。

待真实安装验收：PCL2 2.12.7.3 和 PCL CE 2.14.5.0 人工打开及目标选择；HMCL 实际安装版本；指定实例直接启动；游戏进程确认。跨根目录与错误版本拦截已有模拟回归覆盖，但不代替真实游戏验收。上述未完成项不会显示为已通过。
