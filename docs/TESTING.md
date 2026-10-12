# 测试记录

## 1.0.0 — Fimel 单 EXE 发行，2026-10-12

`cargo test --all-targets --all-features -- --include-ignored` 的 **106 项集成测试通过**；新增 2 项覆盖单 EXE 内置许可、导出复用及拒绝覆盖不同内容。`cargo fmt --all --check`、`cargo clippy --all-targets --all-features -- -D warnings` 和普通 `cargo build --release --bins` 均通过。

当前源码原生窗口回归 `ui-stage4-88303b23` 的 8 组交互及重启通过，生成夹具的原文件哈希不变。三张网站截图为此构建的中文深色存档列表、中文深色空间分析和英文浅色空间分析，使用演示数据；不包含真实游戏内容。此轮使用 Debug 验收功能，不以其响应耗时描述发布性能。

最终单文件 **MinecraftExplorer-1.0-win-x64.exe** 为 **22,254,080 B（21.22 MiB）**，Windows 文件及产品版本 1.0.0，SHA256：`7E5B53411DA579BF99A457DC5527AC3B0795F30A9605309864B336D717BE9C59`。网站 EXE 与构建产物逐字节一致。内置 gzip 解压后的许可文本与新收集的 345 个锁定 Windows 包、797 份许可与归属文件及项目 MIT 逐字节一致，原文总载荷 4,000,283 B，压缩 337,727 B。

普通发布验证 `release-8d11d190`：首次响应 0.53 s、无启动响应超时，10 次连续响应最大 0.50 ms，总计 2.64 s；网站实际下载源文件 `release-d95faca4`：首次响应 0.52 s、无超时，最大 0.60 ms，总计 2.61 s。两者均通过四个测试入口不存在、完整许可资源嵌入、七尺寸 EXE 图标、窗口图标、独立缓存 SQLite v2 及正常关闭。另保留文档 ZIP 于 `dist/MinecraftExplorer-1.0.0/`，网站按钮仅下载 EXE。

Fimel 内容与资源校验、Lint、构建和完整 **57 项浏览器测试通过**。新增测试实际下载文件后核对名称、MZ 文件头及完整 SHA256，并覆盖工具分类搜索、GitHub 入口、三语正文、三个截图、320 px 手机页面和版本页刷新。生产构建在 `/` 及 `/Fimel/` 两个部署目录均通过实际 EXE 下载、同一 SHA256、三张截图加载、手机页面与页签刷新，无页面异常。作品注册细节见 [RELEASE.md](RELEASE.md)。

## 0.4.2 — 设置交互、图标与新增目录自动分析，2026-10-12

`cargo test --all-targets --all-features -- --include-ignored` 的 **104 项集成测试通过**；`cargo fmt --all --check`、`cargo clippy --all-targets --all-features -- -D warnings` 和普通 `cargo build --release --bins` 通过。最后的界面范围选择修复另由原生窗口回归验证。

语言和主题测试现在通过本程序窗口内的真实指针点击、下拉选项方向键及回车执行，不再仅赋值界面属性：检查立即改变、快速连续切换保留最新选择、重启后保持选择，以及切换后原空间分析快照仍可使用。此前的属性赋值测试没有覆盖下拉回调参数类型错误，这次已补齐。

新增目录测试只在带 `ui-smoke` 功能的构建中替换文件夹选择结果，目录验证、注册、索引及自动分析均走生产实现。覆盖已有来源后添加目录、首次添加很小目录、后续索引更新不清空结果、明确选中新增来源，以及取消扫描后保留来源但跳过自动分析。测试目录原文件哈希保持不变，没有写入真实游戏目录。

| 最终原生回归 | 结果 |
| --- | --- |
| Release，150% DPI，1001 个生成世界，`ui-stage4-e6334efc` | 8 组交互及重启通过；交互 9.20 s / 峰值 69.58 MiB / WM_NULL 最大 47 ms；重启 2.41 s / 52.00 MiB / 32 ms |
| Release，首次添加小目录，`ui-stage4-3d4bb16d` | 自动索引、73 B 分析结果、明确来源选择、后续更新保持结果及重启设置通过；交互 0.95 s，重启 0.34 s；短测试未取得有效 WM_NULL 响应样本 |
| Debug，普通 DPI，`ui-stage4-15b3ca40` | 完整交互及重启通过；Debug 响应明显慢于发布版，不用其数据描述发布性能 |
| 普通 Release，`release-fb262593` | 四个测试入口均不存在；全新 SQLite v2、正常退出、EXE 七尺寸图标资源及原生标题栏 HICON 通过；启动首次响应 0.23 s，10 次连续响应最大 1.08 ms |

自动分析期间只有一个后台任务，取消不会随后再开启分析。目录范围比较按当前范围实际包含的物理路径归一化，避免实例发现使来源模型更新时清掉刚完成的小目录分析。原生回归曾暴露这两个问题，修复后使用最终源码重新运行上述测试。图标验收最初错误地查询 `ICON_BIG`；查阅本地 Winit 实现后改为其实际设置的 `ICON_SMALL`，随后通过，没有降低有效性断言。

原创图标为草方块与放大镜组合，嵌入 16、24、32、48、64、128、256 像素 Windows 图标资源，另嵌入窗口 PNG。普通 EXE 文件及产品版本为 0.4.2，生成图标的 Python/Pillow 不属于运行依赖。

最终发布在独立目录 `dist/MinecraftExplorer-0.4.2/`，保留旧版目录。EXE 为 **21,908,480 B**，SHA256：`39BE9AFCE38BDF022F65440940805DD6D764011DFF9FEB58D5D20A793B5A6B75`。打包保留 345 个依赖包的许可与归属。包内 EXE 再运行 `release-fc09bdd5`：首次响应 0.50 s，无启动响应超时，10 次连续响应最大 0.65 ms，图标、全新缓存及正常关闭通过，总计 2.61 s。ZIP 的 CRC、所有随包文档及许可与源码逐字节一致、EXE 与构建产物一致、SHA256 及无测试入口检查均通过。

## 0.4.1 — 扫描与启动入口修复，2026-10-12

`cargo test --all-targets --all-features -- --include-ignored` 的 103 项通过，再加入并单独通过并行内容元信息取消/重试测试 1 项，合计 **104 项集成测试通过**。`cargo fmt --all --check`、`cargo clippy --all-targets --all-features -- -D warnings` 和常规 `cargo build --release --bins` 通过。新增覆盖子树统计复用、并行合并/钻取/归属、取消下限、活动祖先目录无法被改名替换、脚本持久化和旧映射兼容、特殊字符脚本路径及工作目录、失效脚本与跨根目录拒绝。

真实游戏文件仅用于读取。最终 Release 库只读扫描 `D:\PCL2\MinecraftPcl`，报告 `.test-artifacts/perf-0.4.1.json`：

| 指标 | 最终结果 |
| --- | --- |
| 发现条目 | 2257，16.44 s |
| 首个版本核心大小 | 16.99 s |
| 全部游戏目录大小 | 38 个，33.81 s；未知大小 0 |
| 完整扫描 | 48.91 s，未取消 |
| 空间分析 | 442215 个文件，7.14 s |
| 空间逻辑字节 | 58,316,263,022 B |
| 完整性 | 跳过 3 项，明确为下限；100000 个绘图节点 |

同一目录的中间实现（串行内容声明、共享游戏目录末尾统计）为 95.76 s 扫描；最终优先补齐全部游戏目录，并用最多 4 个线程读取内容声明后为 48.91 s。空间单线程中间实现 31.01 s、初次并行 10.61 s、最终 7.14 s，三次读取的文件数与总字节相同。它们是顺序运行的本机读数，受磁盘缓存和其他负载影响，不当作严格的冷缓存 A/B。0.4.0 旧记录为 88.46 s / 456131 文件，目录内容已有变化，不能宣称精确的倍速。没有与 WinDirStat 二进制做同条件对比，也没有实现其整盘 NTFS MFT 加速。

发布版原生窗口另用用户索引的只读备份测试，关闭测试副本的监控，并仅在副本清空实例大小，以验证实际补齐而非命中旧值。`ui-real-patch-533ad440`：38 个目录全部有大小，约 52.38 s（含启动与关闭），峰值工作集 60.12 MiB，正常关闭；成功的 WM_NULL 最大响应约 421 ms，启动期另有一次 2 s 检查超时，因此不宣称全程无卡顿。原索引与真实游戏内容没有写入。

原生界面验收使用生成目录和无害脚本：

- `ui-stage2-79b20644`：29 项操作检查加 5 项重启检查通过，实际保存关联并执行生成的 CMD，工作目录正确；原游戏夹具与假启动器哈希不变，没有运行真实启动器或 Minecraft。
- `ui-stage4-adbdd4c5`：1001 个生成世界，六组树图、双语、主题、列表联动及快捷键检查通过；交互 7.16 s / 51.21 MiB / WM_NULL 16 ms，重启 3.27 s / 49.21 MiB / 32 ms。
- `ui-stage4-76185367`：150% DPI 同样检查通过；交互 7.44 s / 72.84 MiB / 47 ms，重启 5.25 s / 54.40 MiB / 1484 ms。并行编译与 GUI 验收同跑时曾触发 2 s 响应检查失败，停止并行构建后复测通过，未隐藏这些失败。
- 普通 Release `release-8db2c657`：无四个 UI 测试入口，全新缓存启动、SQLite v2 与关闭通过，2.28 s，WM_NULL 最大约 0.49 ms。

最终普通 EXE 为 21,793,280 B，SHA256：`DA3C7D991EDEECB91CC75BB4EB81DC90CDDC4D3B00F11EE0F203CA3A341C7AB6`。发行路径使用独立的 `dist/MinecraftExplorer-0.4.1/`，保留用户正在运行的旧 EXE；ZIP 同版本命名。README 包含完整上手和脚本教程。真实 Minecraft 的用户导出脚本尚未验收；原生启动器指定实例 CLI 与游戏进程确认仍未启用。下文保留各阶段历史记录。

打包后的 EXE 完成 `release-7b33a8b2` 验收：首次响应 2.70 s，启动期有 2 次 1 s 检查超时，随后 10 次连续运行响应均通过，最大 3.12 ms，全新 SQLite v2 缓存及正常关闭通过，总计 4.80 s。验收脚本现在区分“HWND 已可见”和“消息循环首次响应”，启动就绪仍限 20 s；就绪后的响应检查仍逐次限 1 s。首次初始化不是稳定运行阶段，不再混淆两项指标。ZIP 校验 CRC、EXE 与构建产物逐字节一致、SHA256、全部文档与项目许可一致，且四个测试入口字符串均不存在。

## 阶段 4 / 0.4.0 — 2026-10-12

环境仍为 Windows 11 x64 / Rust 1.99.0 / MSVC 14.44。完整运行 `cargo test --all-targets --all-features -- --include-ignored`：**96 个集成测试通过**（阶段 3 的 86 + 空间分析 7 + 双语/设置 3）。之后针对枚举元信息优化又运行空间与双语的 10 项回归，全部通过。`cargo fmt --all --check`、`cargo clippy --all-targets --all-features -- -D warnings` 与常规 Release 构建通过。

空间测试覆盖实际文件总量、重叠根目录与共享目录去重、外部实例目录、最近类别、离线来源不影响正常来源、取消后大小下限、零字节和空目录、junction 拒绝、1000 个条目的矩形合并、面积比例、边界及不重叠。双语测试核对所有界面词条、模板参数数量、动态中文名称/路径保留、旧设置默认值及语言/主题持久化。项目自有源码、测试及脚本未发现纯英文注释；第三方许可原文未改动。README 与 docs 内本地文档链接检查通过。

阶段 4 原生界面使用 1001 个生成世界与重叠来源，执行真实 Slint 指针双击钻取、树图合并与内容列表联动、方向键选择、F2/Delete 安全预览、右键动作回调、双语主题切换与重启。切换语言后分析仍是原快照，重新分析才读到测试新增文件；全部原文件哈希保持不变。源和目标均在有标记的测试目录内，没有向其他应用发送鼠标或键盘输入。测试覆盖右键动作回调，不将其写成所有系统菜单视觉状态均已自动验收。

| 最终 Release 原生界面验收 | 结果 | 耗时 | 峰值工作集 | WM_NULL 最大响应 |
| --- | --- | --- | --- | --- |
| 阶段 4 常规 DPI，`ui-stage4-79c692b8` | 6 组交互检查 + 原文件哈希校验 | 7.22 s | 52.1 MiB | 32 ms |
| 同目录重启 | 语言/主题/索引保留 | 3.50 s | 48.4 MiB | 31 ms |
| 150% DPI，`ui-stage4-a36504c7` | 同样的交互检查通过 | 7.34 s | 70.3 MiB | 32 ms |
| 150% DPI 重启 | 设置保留 | 3.36 s | 53.0 MiB | 31 ms |
| 阶段 2 回归，`ui-stage2-8eb2a96d` | 26 项交互 + 5 项重启 | 18.20 / 2.87 s | 40.0 / 35.9 MiB | 8.88 / 1.02 ms |
| 阶段 3 回归，`ui-stage3-2f24d6a1` | 9 项安全操作检查及关闭收尾 | 3.31 s | 44.2 MiB | 此次未取得窗口探测样本 |

后两项在本阶段空间统计优化前的 Release 界面构建上执行；优化后重新完成阶段 4 的常规/150% DPI 原生验收和空间回归。随后调整快捷键大小写兼容与导航滚动复位，在 `ui-stage4-9902cf68` 的最终源码 Debug 构建上再次完成原生 Ctrl+A/Ctrl+Shift+A、多选上限、方向键、F2/Delete、树图联动及设置重启验收。该 Debug 测试的耗时和响应不作为 Release 性能指标。界面 Release 验收构建保留在 `.test-artifacts/stage4-ui.exe`；正常发行版不含四套测试入口。

对 `D:\PCL2\MinecraftPcl` 及其重叠 versions 路径进行只读空间压力验收，优化后的 Release 诊断统计 **456131 个文件、100000 个保留节点、58902017816 个逻辑字节、3 个未读取项，耗时 88.46 s**。结果明确为大小下限，类别字节之和等于总量。目录元信息枚举不打开游戏文件内容，目录打开前仍检查链接；没有执行真实世界重命名、复制、移动或删除。报告位于 `.test-artifacts/space-real-report.json`。数据量、磁盘及外部负载会影响耗时，不能将小型界面验收的秒数当作真实大目录扫描时间。

PCL CE 的实际 EXE 路径设有 RUNASADMIN，普通进程创建实测返回 740。经用户明确要求打开，修复后的 Rust 调用成功走系统 Shell 分支，取得仍在运行的句柄；真实窗口标题与响应正常。仅确认 OpenLauncher，未验收指定实例或 Minecraft 游戏启动，也未自动化测试交互式 UAC 取消。细节见 [启动器记录](LAUNCHER_RESEARCH.md)。

最终常规 EXE 为 **21520384 字节**，SHA256 为 `7EBEEC24917CF84C4D32E730E89AFDC48B6AC9BFC83E9073CF49C8043DFE2700`。正常构建新缓存启动/关闭通过（`release-7d11de56`，2.41 s，最大响应 0.69 ms）；发布目录 EXE 的同样验收通过（`release-1acbe2f8`，2.47 s，0.58 ms）。两者都确认无四套界面测试入口，SQLite v2 全新缓存可用。压缩包 CRC 校验通过，保留 341 个实际 Windows 依赖包的许可原文与署名；项目文档位于发布目录 docs/。

```powershell
.\scripts\cargo.ps1 -CargoArgs @('test','--all-targets','--all-features','--','--include-ignored')
.\scripts\cargo.ps1 -CargoArgs @('build','--release','--features','ui-smoke')
E:\python\python.exe scripts/ui_stage4.py target/release/minecraft-explorer.exe
$env:SLINT_SCALE_FACTOR = '1.5'
E:\python\python.exe scripts/ui_stage4.py target/release/minecraft-explorer.exe
Remove-Item Env:\SLINT_SCALE_FACTOR
.\scripts\cargo.ps1 -CargoArgs @('build','--release')
E:\python\python.exe scripts/release_smoke.py target/release/minecraft-explorer.exe
.\scripts\package.ps1 -Python E:\python\python.exe
```

兼容性边界：本机 Windows 11 x64 和 150% DPI 已验收，干净 Windows 10/11 虚拟机仍未验收；VC++ x64 运行库依赖保留。Slint 与系统控件采用逻辑尺寸，空间和设置页在较小窗口可滚动。真实指定实例直接启动仍为 OpenOnly；网络/可移动盘回收、热备份、磁盘满及所有显示缩放组合仍未验证。以下阶段 1–3 的数值为历史记录。

## 阶段 3 / 0.3.0 — 2026-10-12

本阶段在同一 Windows 11 x64 / Rust 1.99.0 / MSVC 14.44 环境验收。`cargo fmt --all --check`、`cargo check --all-targets --all-features`、`cargo clippy --all-targets --all-features -- -D warnings` 已通过。自动化运行 `cargo test --all-targets --all-features -- --include-ignored`：**86 个集成测试全部通过**（原 61 + 数据包结构 4 + 文件操作 21）。默认不传 `--include-ignored` 时原生回收验收会跳过，不能把跳过项计为默认测试通过。

数据包覆盖目录/ZIP 同时具有根级 pack.mcmeta 和 data、ZIP 隐式目录及空目录、不完整或嵌套包、普通 .git/tools、损坏 ZIP、删除 data 后的增量清理及恢复。旧误识别缓存会在发现结束时清除，先于世界大小统计；旧注释仍可在同路径重新成为数据包后恢复。

文件操作覆盖：源/目标类别与根目录保护、重命名及带 session.lock 的关闭世界、完整复制校验、冲突保留两份、确认后目标出现、确认后源变化、复制中源变化、取消后的暂存清理、部分批次失败、目录/归档备份恢复及损坏清单拒绝、运行锁、junction、源注册失效、写权限拒绝、注释及世界内数据包注释迁移、崩溃残留目录发现与操作记录展示。版本核心预览不包含额外配置、无关 JAR、独立存档或共享内容。

原生回收验收只处理测试创建的临时条目，实际调用 Windows IFileOperation，核对源消失、对应 `$I` 原路径记录及 `$R` 内容并还原；只移除该测试精确匹配的回收元信息。验收包含关闭存档（含 session.lock）、直接持有真实独占 session 锁时 Shell 拒绝回收、校验后移动、C:→E: 跨盘移动、版本 JSON/JAR 核心回收保留存档，以及单独选择整个已确认独立实例目录的回收/还原。永久删除标志在回调保护测试中被拒绝；没有修改系统回收站配置，没有清空回收站，没有操作真实游戏文件。

```powershell
.\scripts\cargo.ps1 -CargoArgs @('test','--all-targets','--all-features','--','--include-ignored')
.\scripts\cargo.ps1 -CargoArgs @('build','--release','--features','ui-smoke')
E:\python\python.exe scripts/ui_stage3.py target/release/minecraft-explorer.exe
E:\python\python.exe scripts/ui_stage2.py target/release/minecraft-explorer.exe
# 发行前必须重新构建普通版本，排除测试入口：
.\scripts\cargo.ps1 -CargoArgs @('build','--release')
```

阶段 3 Release 原生窗口通过 9 项检查：原生指针选中、确认勾选与陈旧参数保护、重命名及注释同步、复制与多选、同名批量备份、完整校验恢复、不覆盖原件、操作记录、回收预览取消，以及窗口关闭时取消并完成清理。最后的关闭由应用自身处理，Python 验证进程正常退出、原件不变、没有未提交暂存目录。记录 `.test-artifacts/ui-stage3-14e8f339/`：运行 3.63 秒、峰值 RSS 39.88 MiB、WM_NULL 最大 16.00ms。本轮未点击实际回收确认；实际回收由上面的原生临时条目测试独立验收。

阶段 2 Release 回归通过 26 项功能检查及 5 项重启检查，记录 `.test-artifacts/ui-stage2-10090a17/`：分类与图标、搜索标签、收藏最近、监控增删改、重复候选及 PCL CE 配置保持正常。测试只分派事件到本程序窗口，不向其他应用发送输入，不启动真实启动器或 Minecraft。

首次阶段 3 界面测试暴露参数变化后旧确认短暂保留，已增加执行入口的参数签名复核；另一轮暴露程序自己持有 session 锁导致关闭世界不能重命名/回收，已调整委托前的锁释放并复验，同时验证真实独占锁仍拒绝操作。失败轮次未计入通过结果。Debug UI 也通过，但软件渲染响应时间显著高于 Release，不能用 Debug 数据描述发行性能。

限制：没有干净 Windows 10/11 虚拟机、网络/移动盘回收或磁盘耗尽实测；跨盘、权限及失败保留在本机临时目录验证。文件操作不保证对运行中的 Minecraft 做热备份，也不自动重放崩溃任务。真实游戏目录未进行写操作验收；PCL2/HMCL/PCL CE 仍只验证 OpenOnly，直接指定实例启动未完成。阶段 4 Treemap 和界面打磨待开发。

最终普通 Release 已构建，19,213,824 字节（18.32 MiB），无三个 UI 验收入口。全新缓存启动、窗口响应、SQLite v2 空索引和正常关闭通过，记录 `.test-artifacts/release-190d13e9/`：2.42 秒，WM_NULL 最大 0.69ms。发行包使用同一 EXE，包含 341 个依赖的许可归属并验证 ZIP CRC；SHA256：`9EFDCB104A32AFBBC402BD6C538A4DC356E833C54AAAD56F402423ADDE0C4EFF`。这组空缓存指标与实际大目录扫描性能不同，不能混用。

## 阶段 2 历史验收记录

以下为 0.2.0 的原始验收与真实目录只读扫描记录；旧二进制大小、哈希、阶段状态不代表 0.3.0 发行包。

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
