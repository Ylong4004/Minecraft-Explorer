# 测试记录

日期：2026-10-11；阶段 2 / 0.2.0。本机 Windows 11 家庭版 x64，Build 26200。Rust 1.99.0 / MSVC 14.44 / Windows SDK 10.0.26100.0，Slint 1.18.1 software + winit。

## 构建和自动化

已实际执行并通过 `cargo fmt`、`cargo check --all-targets --all-features`、`cargo test --all-targets --all-features`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo build --release`。

共 **61 个集成测试通过**：core 20、inspect 1、process 1、content 14、launchers_ce 4、stage2_index 13、stage2_scan 8。所有修改、损坏、ACL、junction、通知与模拟进程测试使用临时目录。

新增回归覆盖：新旧 pack 格式/文本组件、Fabric/Forge/NeoForge/旧 mcmod.info、多模组声明、ZIP64/中央目录/条目数与解压限制、恶意路径/加密/重复条目、PNG 尺寸限制、v1→v2 迁移/拒绝未知新格式、收藏标签最近持久化、同路径恢复、只读完整 BLAKE3/读取中变化与取消、原生通知增删改名/离线恢复/根重建、四类内容归属去重、单叶增量/实例大小失效、图标独立修改时间，以及 PCL CE 多安装与损坏入口不自动运行其他安装。

| 场景 | 验证结果 |
| --- | --- |
| 共享目录、两个隔离实例的同名世界 | 共享世界一次，同名世界保留不同路径 |
| 多根目录、自定义 gameDir、明确外部归属 | 根/版本/游戏目录分开关联 |
| gzip、原始 NBT、缺失、损坏、level.dat_old | 元信息/降级可用，不覆盖原文件 |
| NBT 毫秒时间戳与解压读取上限 | 时间正确，17 MiB 超限样例降级 |
| 中文、空格、超过 260 字符路径 | 文件读取、统计与索引通过 |
| 无权限、离线目录、游戏占用 level.dat | 继续其他来源，保留存档与缓存 |
| 扫描取消后重试、增量刷新 | 取消不清理旧代次，完整重试成功 |
| LIKE `%` / `_` / 引号、排序与多条件过滤 | 参数化查询，文字字面匹配 |
| junction 循环 / 根路径为 junction | 不跟随，避免循环和重复统计 |
| 版本核心 / 实例目录大小 | 区分顶层 JSON/JAR 和游戏数据 |
| 手动映射共享版本、跨根/错游戏目录 | 明确选择，拒绝冲突映射 |
| PCL2 EXE、HMCL EXE / JAR、缺失 Java | 独立参数/工作目录与错误校验 |
| 实际启动临时 EXE / 模拟 Java 入口 | 参数与 cwd 被子进程真实记录 |
| inspect 接受 versions 与拒绝游戏内缓存 | 输出目录之外读取游戏，不创建游戏内缓存 |

核心压力测试：1000 个临时世界扫描 **5.398 秒**；索引批处理 **132.8ms**；20 次搜索 + 排序合计 **169.9ms**。这是本机 Debug 自动化测试结果，同期存在 Release 编译，含文件系统实际读取，不保证其他磁盘速度。

复现：

```powershell
.\scripts\cargo.ps1 -CargoArgs @('test','--all-targets','--all-features','--','--nocapture')
.\scripts\cargo.ps1 -CargoArgs @('clippy','--all-targets','--all-features','--','-D','warnings')
```

## 原生窗口验收

`ui-smoke` 为单独启用的测试功能，普通发行程序不包含测试入口。测试通过 Slint 公共 WindowEvent 分派鼠标/键盘到自己的原生窗口，核对真实查询结果和 UI 属性，再保存软件渲染器截图；不向其他应用发送输入，不争夺用户鼠标。Python 仅监测此进程/窗口及生成临时结构。

```powershell
.\scripts\cargo.ps1 build --release --features ui-smoke
python scripts/ui_smoke.py target/release/minecraft-explorer.exe
python scripts/ui_stage2.py target/release/minecraft-explorer.exe
python scripts/ui_real.py target/release/minecraft-explorer.exe .test-artifacts/stage2-real/cache/index.db
# 验收后重新构建普通发行程序
.\scripts\cargo.ps1 build --release
```

Python 脚本需要 Pillow、psutil（只用于测试）。本机 Python 位于 `E:\python\python.exe`。

最终 Release 的 1004 世界窗口验收通过：搜索“冒险”返回两个不同隔离实例；世界行图像加载成功、详情归属正确、未配置启动器时禁止打开；切换设置、AboutSlint 对话框、取消/重试后仍有 1004 存档。运行 15.34 秒，峰值 RSS **48.7 MiB**，WM_NULL 响应最大 **6.80ms**。这些指标包含扫描、软件渲染与测试截图，不能当作空闲 CPU。最终记录在 `.test-artifacts/ui-2991ed9a/`。

图标调整后的首次压力复验，测试采样间隔太长而漏过快速完成的扫描，取消步骤等待超时；已缩短该步骤的轮询间隔并重跑，上述最终用例包括真实取消与重试，不把失败轮次计作通过。

阶段 2 fixture 完成 26 项功能及 5 项重启检查：四类内容分类与可见行图标、可信元信息、原生搜索/选择、收藏标签筛选与持久化、最近顺序、重复候选、无 F5 的新增/修改/删除、PCL CE 类型索引 2 保存及重启读取、刷新不覆盖注释。前后核对 fixture 全文件 SHA256，监控临时包删除后原文件树恢复；模拟 EXE 未执行。最终记录在 `.test-artifacts/ui-stage2-fac7fb20/`，功能过程峰值 RSS 36.5 MiB、WM_NULL 最大 11.38ms，重启峰值 32.5 MiB。

阶段 1 Debug 同场景曾通过；本次 Debug 真实目录窗口诊断出现一次 100 秒超时，未计作通过。仪器在 UI 线程生成截图，性能与发布结论采用实际 Release 验收。尚未测高 DPI 多显示器、持续缩放拖动、百万级条目或干净 Windows 10。

## 用户真实目录（只读）

输入 `D:\PCL2\MinecraftPcl\versions`，扫描器使用父根目录 `D:\PCL2\MinecraftPcl`。本次完整扫描缓存/报告存到项目 `.test-artifacts/stage2-real/cache/`。

| 指标 | 结果 |
| --- | --- |
| 存档目录 | 206 |
| 版本目录 / 定义条目 | 40（其中 2 个无唯一 JSON，明确标记未知） |
| 实际游戏目录 | 38 |
| 资源包 / 模组 | 198 / 1081（包含 `.jar.disabled`） |
| 光影包 / 世界数据包 | 63 / 672 |
| 世界图标 / 内容声明预览 | 192 / 1012；声明预览不表示全部图像均已解码验收 |
| 有效 NBT 游玩时间 | 195 |
| 存档逻辑字节总和 | 32,298,677,537（约 30.1 GiB） |
| 完整扫描耗时 | 615.89 秒（Debug，同期有构建与界面测试） |
| 大小未完整条目 | 1 个根目录 Instance，为下限；所有世界大小完成 |
| 来源发现警告 | 0；元信息降级详见各条目 |

最近世界识别为 **Roguelike Bio**，路径：

```text
D:\PCL2\MinecraftPcl\versions\Roguelike Bio-肉鸽生化[导入包] v1.0\saves\肉鸽生化 v1.03
```

该世界本次大小 240,381,441 字节（229.2 MiB）。11 个目录未取得有效 NBT 时间，显示替代信息；没有尝试修复或覆盖文件。

复制真实索引到独立缓存做原生窗口验收：搜索“肉鸽”准确返回此世界；列表及详情图标、真实路径、实例归属、手动映射目标 ID 正确；PCL2 显示 OpenOnly、打开入口可用、真正直接启动保持禁用。世界行图像必须成功加载才进行截图，实际截图已查看。后台真实目录刷新过程中搜索和页面切换成功，随后取消刷新；最终记录在 `.test-artifacts/real-ui-18d2a971/`。没有打开 PCL2 或启动 Minecraft，没有修改游戏文件。

真实截图检查发现最初图标请求将索引暂态读取问题折叠为缺图缓存，已改为可见行路径快照直接在后台读取；刷新清除缺图缓存，图标声明路径纳入缓存键，异步结果同步所选详情。固定占位图尺寸后，实际真实地图列表图标显示正常。

## 启动器真实安装与能力边界

只读确认本机 PCL2 **2.12.7.3** 和 PCL CE **2.14.5.0** 的文件版本；没有做真实启动器/游戏启动验收。HMCL 未找到可验收安装。临时入口进程测试验证本程序的进程参数，**不等价于 PCL2/HMCL 游戏成功启动**。

当前能力 OpenLauncher；SelectInstance / LaunchInstance / TrackGameProcess 未验证，因此不开放。已检查当前公开文档及指定源码快照，记录见 [LAUNCHER_RESEARCH.md](LAUNCHER_RESEARCH.md)。实际安装的人工打开、账号环境复用、指定实例/根目录选择和游戏进程检测仍是后续验收项。

资源管理器定位采用原生 Shell PIDL API，已编译并检查 API 返回错误处理；长路径实际资源管理器选中行为、剪贴板与 F5/Ctrl+F 等系统交互还需要人工验收。没有把这些未覆盖项计作自动化通过。

## 发布检查与限制

普通 Release EXE 18,302,976 字节，约 17.5 MiB，依赖 Windows 系统 DLL 和 VCRUNTIME140.dll / UCRT API；运行环境说明见 README。发行目录包含源码构建说明、测试记录、阶段状态、项目 MIT 与第三方许可、SHA256。源码根目录及 Cargo.lock 一并保留。

普通 Release 的独立副本完成全新缓存启动、窗口响应、SQLite v2 空索引和正常关闭验收；最终打包并改名的 `dist/MinecraftExplorer/MinecraftExplorer.exe` 也通过同一流程，确认没有两个 UI 测试入口，最终记录在 `.test-artifacts/release-9b0b8aa3/`。ZIP 验证 CRC、关键文件并拒绝包含测试入口的二进制；许可清单保留 341 个 Windows 依赖图条目（包括构建/开发依赖）。二进制 SHA256：`F65FB20249549763EE351EECECDF997190EF53D0BD416C5277BA2E007E40EF01`。

阶段 2 已加入文件监控、四类内容元信息、列表图标、注释/最近与只读重复候选。尚不支持写操作、备份/回收站、Treemap。安全文件操作相关的根目录删除/冲突回滚测试将在实现写操作阶段加入。不要将占位导航页当作已完成内容。
