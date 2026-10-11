# Minecraft Explorer

Windows 原生 Minecraft Java 存档与内容索引，Rust + Slint + SQLite。当前版本 **0.2.0** 在阶段 1 MVP 上实现阶段 2 的全局内容管理，并增加独立的 **PCL CE** 启动器类型。PCL2、HMCL、PCL CE 当前均为 `OpenOnly`，指定实例直接启动尚未验证。

## 运行

打开 `dist/MinecraftExplorer/MinecraftExplorer.exe`。面向 Windows 10/11 x64；SQLite 与 Slint 软件渲染器编入程序，无需安装 Rust 或 Java 来运行本程序。依赖 Windows 系统组件及 **Microsoft Visual C++ x64 运行库（VCRUNTIME140.dll）**；若提示缺少此 DLL，请安装 [Microsoft 官方 VC++ Redistributable](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist)。尚未在干净的 Windows 10/11 虚拟机验收，不承诺完全零系统依赖。HMCL JAR 自身需要匹配的 Java。

1. 设置 → 添加 Minecraft 根目录。你提供的测试位置应选择 **`D:\PCL2\MinecraftPcl`**，即 `versions` 的父目录。
2. 也可添加多个根目录，或添加自定义游戏目录，并明确指定其归属根目录。
3. 在全部存档、资源包、模组、光影包、数据包页面搜索名称、文件夹名、实例 ID、完整路径及文件内声明的元信息；支持名称、大小、游玩/修改时间、最近打开和所属目录排序，并按实际游戏目录筛选。数据包从各世界的 `datapacks` 发现。
4. 选中条目查看真实路径、元信息来源、空间占用和版本归属。列表及详情按需加载世界 `icon.png`、包 `pack.png` 或模组声明的 PNG 图标；缺失或损坏时显示占位图。可复制路径或在资源管理器中定位。F5 完整刷新；Ctrl+F / Ctrl+K 聚焦搜索；Esc 关闭弹层。
5. 详情中可收藏、保存逗号分隔的标签，并在收藏、最近打开页面查看。标签筛选匹配完整标签。查看详情、成功发送定位请求、成功创建所选启动器进程会记录最近打开；这些记录只写本程序数据库。
6. 目录监控默认开启，可在设置关闭。变化合并后重新发现条目，仅刷新受影响内容的元信息和大小；所属游戏目录的总大小显示待刷新，用 F5 重新计算。版本 JSON、官方实例配置变化或事件丢失会触发完整刷新。
7. 重复检测由用户主动开始，先按文件大小筛选，再读取整个文件计算 BLAKE3；只列出资源包、模组、光影包、数据包文件的重复候选，支持取消。文件夹和世界不参与哈希，也不提供自动清理或删除。
8. 设置中添加 PCL2、HMCL 或 PCL CE 安装，指定程序文件、工作目录和 Minecraft 根目录。PCL CE 独立保存安装与关联；PCL2/PCL CE 使用 EXE，HMCL 可使用 EXE 或 JAR，JAR 另需选择运行 HMCL 的 `java.exe` / `javaw.exe`。世界/版本详情中的“设置关联”可保存具体版本 ID、实际目录和此实例的默认启动器，同实例的其他内容继承该选择。

“打开所选启动器”使用指定安装启动原启动器，由它处理账号和运行设置。当前为 `OpenOnly`：请在启动器内手动选择所示目标 ID；**打开启动器不表示选中版本或启动游戏**。共享世界不猜测目标版本，需手动关联。失败时可手动切换到关联同一根目录的其他安装，程序不会自动打开另一安装。真实启动器指定实例启动验收仍待完成，详见 [接口核查](LAUNCHER_RESEARCH.md)。

## 数据与操作边界

配置与缓存默认位于 `%LOCALAPPDATA%\MinecraftExplorer\index.db`；`MINECRAFT_EXPLORER_DATA` 可指定独立缓存目录，请与游戏原文件分开放置。索引格式从阶段 1 的 v1 事务迁移至 v2，保留原配置与条目；高于 v2 的未知格式会拒绝打开。收藏、标签与最近打开记录独立保存，扫描不会覆盖它们，路径暂时消失或移除后重新添加同一路径仍可恢复注释。“移除目录”仅移除本程序关联与索引。

世界名/游玩时间优先来自 gzip 或未压缩的 `level.dat`，必要时只读尝试 `level.dat_old`；缺失/损坏时明确标注目录名和文件时间为替代信息。世界历史版本不是启动目标。跳过符号链接/junction/重解析点；缺权限时大小显示下限。逻辑大小不是磁盘实际分配空间；世界已包含在其游戏目录大小内，不能重复相加。离线目录保留缓存。

资源包/数据包读取 `pack.mcmeta`；模组读取 Fabric、Forge、NeoForge 和旧 `mcmod.info` 声明。显示名、描述、ID、版本、作者及兼容范围来自这些文件，不根据文件名猜测，也不把声明范围当作兼容性检查。光影包缺少统一元信息时保留文件名、路径和明确提示。ZIP/JAR 仅读取指定元信息与小图标，限制读取、解压和中央目录大小，不解压到游戏目录；损坏、不支持、加密或超限的条目仍保留可查路径。

本程序的扫描、图标预览和重复检测不修改游戏内容，不上传文件清单或存档数据。打开原启动器后，它会执行自身正常行为。阶段 3 的重命名、复制、移动、回收站、备份及阶段 4 的 Treemap 尚未实现；请继续使用现有备份工具。暂时请对同一缓存运行一个程序窗口。

## 构建

稳定 Rust（`rust-version = 1.92`）、Visual Studio C++ Build Tools 和 Windows SDK。当前验证为 Rust 1.99.0、MSVC 14.44、Windows SDK 10.0.26100.0。Slint 固定 1.18.1，采用 winit + software，未引入 Electron/WebView。

```powershell
cargo fmt --check
cargo check --all-targets --all-features
cargo test --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
```

本工作区还提供隔离工具链包装脚本（不修改全局 PATH）：

```powershell
.\scripts\cargo.ps1 build --release
.\scripts\cargo.ps1 -CargoArgs @('test','--all-targets','--all-features','--','--nocapture')
```

输出 `target/release/minecraft-explorer.exe`。发布时复制到 `dist/MinecraftExplorer/MinecraftExplorer.exe`，并附文档、第三方许可文本；`scripts/package.ps1` 执行打包与 SHA256 生成。

只读命令行扫描（缓存必须放到游戏根目录之外）：

```powershell
.\scripts\cargo.ps1 -CargoArgs @('run','--release','--bin','inspect','--','D:\PCL2\MinecraftPcl\versions','.test-artifacts\real-pcl2')
```

该工具接受根目录或 `versions` 并使用同一扫描器，输出 `report.json` 和独立索引。桌面添加目录时直接选择根目录。

项目新编写源码为 MIT；Slint 采用 Royalty-free 2.0，设置 → 关于包含实际 `AboutSlint` 控件，发行包保留许可与归属。详见 [许可说明](THIRD_PARTY.md)、[架构](ARCHITECTURE.md)、[测试](TESTING.md)、[阶段记录](DEVELOPMENT.md)。
