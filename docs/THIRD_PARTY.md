# 第三方许可与归属

新编写的 Minecraft Explorer 源码采用 MIT，见 [LICENSE](../LICENSE)。原需求文档 `提示词.md` 不因本声明改变其权属。Minecraft、PCL2、PCL CE、HMCL 名称用于描述兼容对象，本程序未包含其二进制、游戏素材或登录组件。

应用图标为项目原创的草地方块与放大镜几何图形，SVG、PNG、ICO 及生成脚本适用同一 MIT 许可，未使用 Minecraft 原版纹理。0.4.2 新增构建依赖 embed-resource 及其锁定依赖，其许可由发行清单保留；Pillow 仅用于可选图标再生成和开发测试，不包含在 EXE 内。

Slint 1.18.1 及对应内部 crates 选择 **LicenseRef-Slint-Royalty-free-2.0** 许可路径；没有选择其商业许可或以名称相似推断 MIT。当前为桌面应用，设置 → 关于嵌入真实 `AboutSlint` 组件，并保留组件中的 Slint 标识/链接。详细条款随包放在 `licenses/`，基于 Cargo 下载的原始 LICENSES 文件。改变产品用途或移除署名时应重新核对原文条件。

发行包 `THIRD_PARTY_NOTICES.md` 由 `scripts/package.ps1` / `collect_licenses.py` 按锁定的 Windows 依赖图生成，列出实际 package/version/SPDX/许可文本路径。为保守保留归属，也收集该图中的构建及开发依赖；清单不表示每个包都静态链接到最终 EXE。MIT/Apache/Unicode/BSD/Zlib 等文本、包内 NOTICE 和第三方文本一起复制。SQLite 源码随 rusqlite bundled 构建；其上游声明亦由包内源码文本收集。

构建与核查依据为 Cargo.lock、Cargo metadata 及 crates 原始许可文件。程序没有复制 PCL/HMCL GPL 源码实现，只阅读其接口入口，研究快照不放进发行包。

从 1.0.0 开始，项目 MIT 与发行包全部第三方许可、归属原文也嵌入 `assets/third-party-licenses.txt.gz`，以支持网站的单 EXE 下载。`scripts/embed_licenses.py` 从已经完成许可收集的目录确定性生成该资源。设置 → 关于 → 导出第三方许可可将完整原文导出到程序数据目录并定位查看；不依赖网络或 EXE 旁的许可目录。压缩只减少体积，不修改上游原文。
