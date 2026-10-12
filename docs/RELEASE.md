# 1.0 单文件发行与 Fimel 登记

官网作品页：`https://studiofimel.com/#/works/minecraft-explorer`。源码：`https://github.com/Ylong4004/Minecraft-Explorer`。

1.0.1 增加 PCL2/PCL CE 上次启动脚本的自动识别与独立副本，作为 1.0 系列修订发布。网站仍使用用户指定的 `MinecraftExplorer-1.0-win-x64.exe` 文件名，实际产品版本和页面版本标注为 1.0.1。

`scripts/package.ps1` 在复制前核对 EXE 的 ProductVersion 与 Cargo 包版本；旧产物不能冒充新版本。打包完成后同步 `dist/MinecraftExplorer-1.0-win-x64.exe` 这一系列入口，默认包为 `dist/MinecraftExplorer/MinecraftExplorer.exe`。历史版本目录保留。该脚本使用 UTF-8 BOM，保证 Windows PowerShell 5.1 正确读取中文提示。

网站分类为“插件与工具”，记录位于 Fimel 的 `public/works/catalogue.json`。概述、更新日志及版本页在 `public/works/minecraft-explorer/`，提供中英日正文；应用本身仅提供中文和英文。封面使用本项目原创图标，三张画廊图片为实际原生窗口显示的生成演示数据，不包含用户真实存档。

网站下载只有 `public/plugins/minecraft-explorer/MinecraftExplorer-1.0-win-x64.exe`。发布文件来自普通 `cargo build --release --bins` 的主程序，不包含 `ui-smoke` 功能。单文件内置图标、SQLite、Slint 软件渲染器以及项目和上游许可文本；Windows 系统组件及 VC++ x64 运行库仍为系统依赖。

## 再次发行

1. 更新 Cargo 包版本、关于页和相应文档，保持版本含义与下载文件名一致。
2. 依赖变化时，先用 Cargo metadata 和 `scripts/collect_licenses.py` 收集锁定 Windows 依赖的许可，再运行 `scripts/embed_licenses.py <收集目录>` 更新 `assets/third-party-licenses.txt.gz`。确定性压缩不修改上游原文。
3. 运行格式、Clippy 与适用测试，再构建普通 Release。用 `scripts/release_smoke.py` 检查独立缓存启动、正常关闭、七尺寸图标、内置完整许可和缺少测试入口。
4. 将主 EXE 复制到 Fimel 指定路径。比较 SHA256 和字节内容，确认关于页及 Windows 文件版本；同步版本正文、目录体积与发行校验记录。
5. Fimel 运行内容和静态资源校验、Lint、构建及浏览器下载测试。测试实际下载后的文件名、EXE 文件头及 SHA256，不能用模拟下载代替发行文件检查。
6. 提交并推送两个仓库，检查既有 GitHub Pages 部署结果。单文件下载保持不需要解压；EXE 超过部署平台单资源限制时重新规划分发，不直接将 ZIP 替换到 EXE 路径。
