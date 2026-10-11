# 架构

## 模块

| 文件 | 职责 |
| --- | --- |
| `ui/app.slint` | 原生窗口、虚拟列表、搜索、详情、设置与关联弹层 |
| `src/app.rs` | UI 状态、后台命令线程、独立扫描线程和消息通道 |
| `src/discovery.rs` | 多根目录、共享目录、隔离内容、自定义 gameDir 的发现 |
| `src/minecraft.rs` | 有读取上限的 NBT / 版本 JSON 元信息解析 |
| `src/content.rs` | 资源包/模组/光影包/数据包的有界 ZIP/JAR/目录元信息与 PNG 预览 |
| `src/scanner.rs` | 可取消迭代遍历、逻辑大小、部分结果与进度 |
| `src/index.rs` | SQLite v2 迁移、配置、持久索引、独立注释、查询与离线缓存 |
| `src/watch.rs` | notify 原生事件、防抖、有界事件队列与失效订阅重建 |
| `src/duplicates.rs` | 按大小筛选后串行全文件 BLAKE3，生成只读重复候选 |
| `src/models.rs` | Source / Item / LauncherInstallation / LaunchMapping |
| `src/platform.rs` | 路径规范化、重解析点检查、原生资源管理器 PIDL 定位 |
| `src/launcher.rs` | 独立 PCL2、HMCL、PCL CE Adapter、能力边界与独立进程参数 |
| `src/bin/inspect.rs` | 使用同一扫描器的只读诊断入口 |
| `src/ui_smoke.rs` | 仅 `ui-smoke` 功能开启的原生窗口集成验收 |
| `src/ui_stage2_smoke.rs` | 阶段 2 分类、行图标、注释、监控、重复检测和配置重启验收 |

## 目录与归属

Root、Version、Instance、World 与 ResourcePack、Mod、ShaderPack、DataPack 分开建模；ID 使用类型 + 规范化真实路径，Windows 大小写与长路径前缀统一。同名世界保留不同 ID。共享 `saves` 只发现一次。版本目录中实际存在 saves/mods/resourcepacks/shaderpacks/screenshots 时标记独立内容目录；这不是对启动器隔离设置的推断。无唯一 JSON 或缺少 ID 则标为未知。基础版本与 Loader 只由 JSON 字段、明确库坐标识别。

每个实际游戏目录枚举 `resourcepacks`、`mods`、`shaderpacks` 的直接子项；资源包/光影包支持 ZIP 与目录，模组支持 JAR、`.jar.disabled` 与目录。每个世界单独枚举其 `datapacks`，并记录所在世界和实际游戏目录。多个来源指向同一物理游戏目录时先去重，避免共享内容重复计数。

官方 `launcher_profiles.json` 仅投影 profiles 的 name/gameDir，不反序列化账户信息。没有解析 PCL/HMCL 私有账户配置。自定义游戏目录的归属由用户选择根目录确认。

NBT 读取及解压均限制 16 MiB；版本 JSON 限制 4 MiB。读取失败保留条目。每次遍历前检查路径各组件，默认不跟随 reparse point；这是保守读取策略，不作为尚未开发的写操作安全保证。

内容解析仅读取已识别的 `pack.mcmeta`、`fabric.mod.json`、Forge/NeoForge TOML、旧 `mcmod.info` 等条目。单个元信息条目上限 1 MiB、一次元信息预算 4 MiB；ZIP 中央目录上限 8 MiB、条目数上限 40,000，多模组声明最多展示 256 条。压缩与解压后的长度均检查，重复归档路径、路径穿越、链接、加密及不支持归档会明确失败，不向游戏目录解压。包描述支持有界文本组件；模组构建占位符保留原声明并提示，兼容范围按原声明展示，不执行兼容性求解。

图标来自世界 `icon.png`、包 `pack.png` 或模组声明的 PNG；读取上限 2 MiB、解码最大 1024×1024、分配上限 8 MiB，缩为最大 96×96。缺失、超限或损坏图标使用占位图，不阻止列表显示。

## 大小口径

World 包含世界完整目录；Version 仅统计顶层 JSON/JAR 核心文件；根 Instance 排除顶层 `versions`；独立/自定义 Instance 排除顶层 JSON/JAR。内容文件按文件长度、目录包按递归文件长度统计。各行大小解释其类型：世界已包含数据包，游戏目录已包含世界及内容，不能相加做总占用。未知扩展核心文件暂未分类。文件不可访问或跳过链接时为可统计的下限；压缩文件、稀疏文件按逻辑长度计算。

## 后台与索引

UI 只接收后台结果。命令线程查询 SQLite 并处理设置、详情及系统操作；扫描另用线程和 SQLite 连接，串行遍历（并发度 1），先发布发现结果，再优先读取世界，最后实例大小。每个文件/目录检查取消；进度节流 200ms，UI 每 60ms 处理消息，搜索防抖 180ms。查询请求带代次，忽略过时结果。虚拟列表使用 Slint ListView，不创建全量绘图控件。

列表行图标由可见 ListView 行请求，单独的后台图标线程按已显示条目的路径快照读取/解码，避免阻塞搜索与设置命令，也不依赖扫描期间的数据库写锁。请求通道容量 64，UI 最多缓存 256 个图像结果，缺失结果也缓存；满队列时延后有限数量请求。缓存键含条目 ID、修改时间、大小、预览修改时间与声明的图标路径，只有当前键一致的结果才更新行与所选详情；内容刷新会使旧键失效，扫描开始清除缺图缓存以允许重试。后台传递 RGBA 字节，Slint 图像及行模型仅在 UI 线程更新。

SQLite WAL / NORMAL，busy_timeout 5 秒。`settings` 单行 JSON 保存 sources/launchers/mappings/default_launcher/instance_defaults/watch_enabled。`items` 包含 id、kind、source、game_dir、name、search、size、time_ms、ownership、available、generation、json。类型+大小/时间、game_dir、source 建索引；模糊子串查询仍需扫描匹配范围，尚未验证百万条数据。

当前 schema version 2；事务迁移保留 v1 的配置与索引，新增字段由 serde 默认值读取，高于已知格式的数据库直接拒绝打开。`annotations` 单独保存 favorite、tags_json、search、last_opened_ms；`annotation_tags` 保存完整标签及规范化键，索引 tag_key/item_id。扫描 upsert 只更新 `items`，query/get 合并用户注释。删除失效条目或移除来源不删除注释，同 ID 恢复后重新合并。标签筛选用完整键匹配，模糊搜索包含标签和可信内容描述/ID/版本，并转义用户输入的 LIKE 通配符。查看详情、定位请求成功、成功创建启动器进程在本数据库记录最近打开，不通过修改游戏文件时间保存记录。

发现时事务写入新条目并标记代次，已有元信息继续可见；扫描完写入实际结果。完整成功的来源才清理旧代次；取消保留旧索引。离线来源标记不可用并保留缓存；局部读取失败不把整个来源标为离线。

## 文件变化与重复候选

notify 监听配置根目录、自定义目录和发现的外部 gameDir，去除已有递归根覆盖的路径。默认不跟随链接，并对事件路径的最近存在父级再做重解析点检查。事件队列有界，900ms 安静期合并，持续写入最长 3 秒发送一次批次；溢出、native 错误及订阅重建标为需要完整发现。失效根低频重试；Windows 卷号/文件 ID 与 Unix dev/inode 检查根目录是否被删除重建，避免订阅停留在旧目录对象。

普通变化先重新发现成员，只读取新增或受影响叶条目的元信息和大小；受影响 Instance 的总大小失效为待刷新，F5 完整重算。版本 JSON、`launcher_profiles.json` 或事件丢失使用完整扫描。仅当来源发现完整且路径明确已不存在时移除缓存条目；取消不进行清理。监控可关闭，手动刷新仍可使用。

重复检测仅在用户请求时执行，对索引中的四种内容文件先按当前文件长度分组，再串行读取完整文件计算 BLAKE3。不对目录或世界哈希，不因同名宣称重复；同一规范化物理路径去重。每次读取检查取消及路径重解析点，读取前后核对大小、mtime 与完整读取字节数，变化或不可读文件报告并跳过。相同哈希作为候选显示，不实施删除或估算已释放空间。

## 启动映射

映射同时保存 subject_id、launcher_id、root、game_dir、具体 version_id、version_path、来源。建立映射检查物理目录、根目录一致、实际独立游戏目录一致。共享目录必须由用户明确选择版本。保存关联时同时记录 subject 与实际 Instance 的默认启动器，同实例内容继承该选择；这些设置可覆盖全局默认。多个安装以独立安装 ID 保存映射，PCL CE 是独立种类，旧 PCL2/HMCL 配置保持兼容。

能力定义为 OpenLauncher / SelectInstance / LaunchInstance / TrackGameProcess。PCL2、HMCL、PCL CE 目前均只公布 OpenLauncher。PCL2/PCL CE 仅接受 EXE；HMCL 可用 EXE 或 JAR，EXE 无额外参数，HMCL JAR 传独立参数 `-jar`、JAR 路径；`Command` 设置明确 working_dir，不通过 cmd 拼接。退出状态仅描述启动器进程，不推断 Minecraft 已运行。失败时用户可手动换同根目录的其他安装，不自动运行另一安装。没有界面自动点击、修改启动器配置或读取认证信息。直接启动仍需按真实安装版本验证公开接口，研究与验收边界见 [LAUNCHER_RESEARCH.md](LAUNCHER_RESEARCH.md)。
