---
AIGC:
    Label: "1"
    ContentProducer: 001191440300708461136T1XGW3
    ProduceID: f60842f40542d18996ccf15309f5e3de_17863382ba6111f1b172525400248c00
    ReservedCode1: YmoiOHAOfG57fZJy69SXZFnCWdnF/AeAB0/Cohw2OgEWa4fKJ+PaGVkwjIpLWIQn6qCv8/2MGtFYjTy8hBmoy//d8jizX1W1nuNeeADux2XaLs+nrQIiODf5xGTS89E0dXt+VWG5wA4lDSCq/QwiXH+P4Mbs2ezL2a6o/6jqAz8btBxvHsJo1MYnTW0=
    ContentPropagator: 001191440300708461136T1XGW3
    PropagateID: f60842f40542d18996ccf15309f5e3de_17863382ba6111f1b172525400248c00
    ReservedCode2: YmoiOHAOfG57fZJy69SXZFnCWdnF/AeAB0/Cohw2OgEWa4fKJ+PaGVkwjIpLWIQn6qCv8/2MGtFYjTy8hBmoy//d8jizX1W1nuNeeADux2XaLs+nrQIiODf5xGTS89E0dXt+VWG5wA4lDSCq/QwiXH+P4Mbs2ezL2a6o/6jqAz8btBxvHsJo1MYnTW0=
# XMST 交接文档（HANDOVER）
> 重建说明：2026-09-27 该文件被意外截断，AIGC 标注占位（无卷影副本、无 git），本版本由 AI 依据截断前保留的会话记录重建。**早期章节（章节 1/2/3/5 与阶段 0-10 的部分变更记录）缺失，待人工补充**；尾部章节（7/8）与阶段 11/8/C/图标相关记录按原样还原。
### 截断原因分析（2026-09-27 意外截断，AIGC 标注占位）：
经排查，本次截断无法从本地快照/版本控制恢复，证据链如下：
1. **无卷影副本（VSS）**：HANDOVER.md 所在卷未启用或未配置卷影复制，`vssadmin list shadows` 无可用快照，无法回滚到截断前版本；回收站中亦无旧版 HANDOVER.md。2. **无 git 版本控制**：F:\XMST 未纳入任何版本库（无 .git），文件被覆盖后无提交历史可回滚，唯一残留为 HANDOVER.md.corrupt_bak（仅含 AIGC 标记空内容，正文未备份成功）。3. **写入链路特征**：该文件由 AI Agent 工具链全量重写落盘（write_file/edit_file 链路，UTF-8 整文件替换），每次写入都会重新生成 AIGC 标注头（ContentProducer/ProduceID/ReservedCode）。本次截断的形态是"标注头完整、早期章节 1/2/3/5 与阶段 0-10 大部分记录丢失、尾部章节 7/8 与阶段 11/8/C 保留"，符合 **生成/写入环节内容截断** 特征（上下文窗口截断或写入缓冲截断），而非用户手动编辑删除——用户手动编辑不会恰好保留下半部分并保留完整标注头。4. **重建依据**：会话交接_2026-09-26.md（阶段 0-7 完整记录）、README.md（2026-09-20/21 变更）、docs 目录、HANDOVER.md 残留尾部及历次会话记录，均用于本次补全；凡无记录佐证的内容（如阶段 9/10）如实标注缺失，不臆造。
> **2026-09-29 文档合并**：HANDOVER_综合交接与经验总档.md 全部内容已并入本文档「第 9 章 综合交接与经验总档」（关键架构决策 / 全部历史错误与经验教训 / 各阶段交付汇总 / 文件存储位置规范 / 后续开发防错清单 / 当前已知问题与待办，含过时点注释），原文件退役归档，此后仅维护本文档 HANDOVER.md。
> **2026-09-29 发布策略变更**：构建产出由双 exe 改为单 exe，命名 `dist\XMST-<版本号>.exe`（版本号取 Cargo.toml version），不再产出双 exe。
## 最近变更记录

- 2026-10-03（第十七轮：夜间模式图标确认 + 目录打开行为回稳）：
  1. **夜间模式图标**：确认右上角主题按钮月亮图标已为**完整圆月**（`circle_filled` 实心圆，半径 5.6，无月牙缺口），交叉淡入淡出与主题色保留；本轮无需再改。
  2. **「打开所在目录」行为回稳（目录问题按用户要求不动）**：`spawn_reveal()` 由 explorer.exe 直开优先**调回 `cmd /C start` 优先**——目录 `cmd /C start "" <dir>`、文件 `cmd /C start "" explorer.exe /select,<path>`，显式 `current_dir(sane_cwd())` + 空标准句柄 + `CREATE_NO_WINDOW`，与旧版 dist（E163C1A6，目录打开正常）行为一致；explorer 直开仅作兜底。目录无法打开问题本身维持现状（已交接 Deepseek 处理，本轮不动）。
  3. **产物**：`dist\XMST-0.1.0-alpha.exe` SHA256 `2826BCEE…`（15,515,136 B；增量 release 3m12s，C 盘 target `C:\Users\xiumi\AppData\Local\Temp\xmst-target` 构建，避开 F 盘 ACL）；dist 旧版备份 `F:\XMST\dist\backup\20261003_131314\XMST-0.1.0-alpha.exe.prev`。

  1. **右上角主题图标改圆月**：亮/暗切换按钮的月亮图标由「新月/月牙（外圆减偏移内圆多边形）」改为**完整圆月**——`circle_filled` 实心圆（半径 5.6），保留交叉淡入淡出与主题色；太阳图标（描边圆+8 光线）与自定义三态（调色板）不变。
  2. **「打开所在目录」全部入口统一修复**：新增模块级 `spawn_reveal()`，目录走 `cmd /C start "" <dir>`、文件走 `cmd /C start "" explorer.exe /select,<path>`（与 `open_folder` 首选同路，实测最稳），显式 `current_dir(sane_cwd())` + 空标准句柄，避开 GUI 进程「当前目录失效 / 标准句柄缺失」导致的 explorer 子进程 0xc0000142；spawn 失败再退化为 explorer.exe 直接拉起。原 `explorer_select`（带 CREATE_NO_WINDOW + 整串参数，注释已证易触发 0xc0000142）、`reveal_in_explorer`（缺 current_dir/空句柄）、`open_file_location` 三处统一委托 `spawn_reveal`；覆盖调用点：Java 所在目录、下载历史/自定义目录、文件浏览器右键「打开所在目录」、崩溃分析「跳转到该位置」。
  3. **产物**：`dist\XMST-0.1.0-alpha.exe` SHA256 `E163C1A6C30405143F7B504C7DA3CC4B16702046A6A62892CAC2580E90609270`（15,499,776 B；增量 release 构建）。源码备份 `F:\XMST\backup\20261003_035334\`（main.rs/theme.rs/features.rs .bak）；dist 旧版备份 `F:\XMST\dist\backup\20261003_041042\XMST-0.1.0-alpha.exe.prev`。


- 2026-10-03（第十五轮：崩溃跳转 / 弹窗可缩放 / 快照过滤 / 页签动效 / 折叠卡顿根因 / 卡片与预览精简）：
  1. **崩溃分析可跳转（用户要求）**：`crashscan::Cause` 新增 `path`（相对服务器目录）——JSON 配置损坏类结论会在 `config/`（含一层子目录）里按模组 id 模糊查找对应配置并给出相对路径，找不到则回退到 `config` 目录；EULA → `eula.txt`，端口占用 → `server.properties`。弹窗里显示 `📄 路径` + **「跳转到该位置」**（资源管理器定位/打开，新增 `reveal_in_explorer()`）。
  2. **弹窗可缩放**：为 16 个 `egui::Window`（创建新服务器、删除/重命名、文件编辑、隧道配置、确认类弹窗等）统一加 `.resizable(true)`。
  3. **创建新服务器：默认隐藏快照版**：`CreateServerState.show_snapshot`（默认 false）+ 搜索框旁「显示快照版」勾选框；`filtered_versions()` 过滤 `-pre/-rc/snapshot/experimental/24w45a` 形态（新增 `is_snapshot_version()`）；无正式版时给提示。
  4. **内网穿透子页签动效**（用户反馈"内部切换无平滑"）：`tunnel_side` 的 5 个页签改为自绘行 + **滑块位置插值**（`tunnel_nav_anim`）+ 悬停底色过渡 + 左侧竖条，与主侧栏同款。
  5. **设置折叠"卡一下"根因**：动画期间 `content_h` 取到的是**被 `max_height` 裁剪后**的高度，却被写回 `last_h` → 动画目标逐帧缩水 → 一顿一顿。改为**只在完全展开（anim≥0.99）或首次未知时**记录真实内容高度。
  6. **概览卡片高度自适应**：去掉 `set_min_height(84)`，改为按内容高度（此前三张卡片都固定 84pt，显得过高）。
  7. **字号预览精简**：只保留「正文示例」四字（删除次要/小字/标题三行），预览框宽度 320→220。
  8. **删除两处文本**：文件浏览 mods 页的「已识别：…」行、插件页标题右侧的「测试功能：rhai 脚本插件…」说明。
  9. **产物**：`dist\XMST-0.1.0-alpha.exe` SHA256 `A52BC5216745649025F4788372E54311E7200AFE7B4D236CFA13844F22C0225F`（15,389,184 B；cargo check 0 error；release 3m08s）。还原点 `git 2bfd6bf` 之前均为本轮改造前状态。
  10. **ExplorerBlurMica 调研结论**：它是 `regsvr32` 注册的 **shell 扩展 DLL + minhook 注入 explorer.exe**（清背景 + 画模糊/亚克力/Mica），只对资源管理器生效；作者另一个项目 **DWMBlurGlass** 才是"全局窗口模糊"（hook dwm.exe）。**两者都是系统级注入，不能给我们的 GL 窗口提供可用 API**；唯一可借鉴的是架构方向——若将来要真·系统模糊，需把呈现改成 DirectComposition + `IDCompositionBackdropBrush`（`WS_EX_NOREDIRECTIONBITMAP`），让内容不再覆盖效果层。

- 2026-10-02（第十四轮：托盘黑屏再修 / 双击唤出托盘实例 / 崩溃根因分析 / 概览卡片化）：
  > **还原点**：`git 3858ab2`（本轮改造前基线）、`git 1fb4b72`、`git 2bfd6bf`；源码整份备份 `F:\XMST\backup\20261002_234713\`（含改造前 exe）。可按文件回退：`git checkout 3858ab2 -- src/main.rs`。
  1. **托盘黑屏（第二轮定位，两处真因）**：
     * ① **清理时序错**：上一版在**关闭帧**就丢纹理/清 egui 缓存，而那一帧的交换缓冲可能还是黑的，且 winit 的 `Visible(false)` 尚未生效 → 桌面上留一个黑窗口。现改为：关闭帧只发 `Visible(false)`（该帧正常合成），**等窗口确实隐藏后**再做清理——清理里先 `ShowWindow(SW_HIDE)` 兜底真正隐藏，再丢弃纹理句柄 + 清缓存；清理每次隐藏只做一次（`tray_cleanup_done`）。
     * ② **winit 状态不一致**：托盘恢复只调了 Win32 `show_main_window()`，winit 仍以为窗口隐藏 → **不渲染 = 恢复后黑屏**。现在恢复帧先 `ViewportCommand::Visible(true)` + `Focus`，再重建字体/壁纸/材质/圆角区域。
     * ③ 保留上一轮的防御：纹理缺失时 `material=None`（回落不透明面板），结构上不可能黑屏。
  2. **双击 exe 自动唤出托盘实例**：新增命名事件 `Local\XMST_ShowMainWindow`。运行实例里起一条线程阻塞在 `WaitForSingleObject`（零 CPU），第二个实例 `OpenEvent+SetEvent` 后静默退出 → 托盘里的窗口自动弹到前台。比 FindWindow+ShowWindow 可靠（后者绕过程序内部状态，正是黑屏成因）。
  3. **崩溃根因分析（新模块 `src/crashscan.rs`）+ 自动弹窗**：服务端异常退出时自动分析 `crash-reports/*.txt` 与 `logs/latest.log`，弹出「⚠ 崩溃原因分析」小窗（结论 / 涉及模组 / 处理建议 / 原始证据行，可折叠），并提供打开日志与崩溃报告目录、重新分析。
     * 规则：依赖缺失、Java 版本过低、内存不足、端口占用、Mixin 冲突、EULA、重复模组、仅客户端模组、世界版本不兼容，以及**通用「异常 + 堆栈归属模组」**。
     * **堆栈→模组模糊匹配**：归一化（去非字母数字、小写）后与 mods 目录里的 mod id 双向包含匹配，跳过 JDK/Minecraft/加载器帧。
     * **实机验证（真实崩溃报告）**：`D:\Desktop\Fabric1.21.11\crash-reports\crash-2026-09-20_10.29.50-server.txt` 输出：
       `结论=模组配置文件损坏（config 里的 json 格式不正确）`、`涉及=Carpet Ayaka Addition（carpet-ayaka-addition）、Carpet Mod（carpet）`（从包名 `com.ayakacraft.carpetayakaaddition` 正确匹配）、并给出"改名/删除该模组配置让其重新生成"的建议 ✓
     * 诊断入口：`set XMST_CRASHSCAN=<服务器目录>` 直接跑分析并打印结论（无需 GUI，便于回归验证）。
  4. **概览卡片化（UI 改造第 1 项）**：名称 + 五个文件夹快捷入口一行；下面三张卡片——**状态**（运行中/已停止/正在停止 + 最后消息 + 目录存在性）、**服务端平台**（类型·MC 版本·加载器版本·模组/插件数，悬停看识别依据）、**目录**（world/mods 体积、eula.txt 是否存在，新增 `dir_size_mb()` 目录体积统计）。
  5. **产物**：`dist\XMST-0.1.0-alpha.exe` SHA256 `B9537F6F95296213A8AF974B96913589221E9092C589CDA20B752DBCFA0EBA68`（15,377,408 B；cargo check 0 error；release 3m07s）。
  6. **UI 改造剩余项（下一步）**：② 服务器列表行内操作（悬停出现 ▶/⏹/📂）；③ 强停/回退/删除统一确认弹层组件；④ 日志面板过滤/高亮/跟随；⑤ 设置页搜索。

- 2026-10-02（第十三轮：托盘黑屏根因 / 启动"警告"根因 / 清理与实测收尾）：
  1. **托盘 → 恢复 → 再关闭 出现黑屏窗口（根因）**：进入托盘态 3s 后为了把工作集压到 ~2MB，会 `mem.caches = 默认` + `set_fonts(默认)` + `EmptyWorkingSet`。这会**让背景材质纹理句柄失效**；而材质模式下内容面板填充 alpha = 0（靠纹理透出桌面）→ 纹理一没，整窗只剩 clear 色 = **黑屏**。**三层修法**：① 隐藏帧显式 `backdrop.forget_texture()` + 丢弃噪点/壁纸纹理句柄（避免往失效句柄 `set()`）；② 新增 `tray_restoring_flag`（托盘回调置位、update 消费）：恢复首帧重置 `cjk_loading`（重载 CJK 字体）、重载壁纸、强制立即重抓材质、失效并重设窗口圆角区域、连发 3 帧重绘；③ **防御**：纹理缺失时 `material = None` → 主题回落常规不透明面板，**任何情况下都不会黑屏**（首帧抓取完成后材质自动出现）。
  2. **每次启动的"打开文件警告"（根因）**：不是 SmartScreen —— 实测 exe **无 Zone.Identifier（无 MOTW）**、`SmartScreenEnabled=off`、清单 `asInvoker`（无 UAC）。真正来源是**我们自己的单实例检查**：`close_behavior=tray` 时关闭窗口只是隐藏，进程仍在，于是**每次双击 exe 都会命中"已在运行"的模态 MessageBox**。改为**静默退出**（并尽力把已可见的旧窗口带到前台），不再打断用户；托盘图标本来就在，点一下即可恢复。
  3. **磁盘**：完成清理后 F:\XMST = 4.09 GB（`target/release` 3.89 GB 为增量缓存）；`target/debug` 与 MinGW 工具链已删（构建工具链实际在 `C:\Users\xiumi\dev\tools\mingw64`）。
  4. **性能实测**（详见第十二轮）：窗口 125–136 MB WS、最小化 125 MB、**托盘 2.4–2.5 MB**、CPU 空闲 0%。
  5. **产物**：`dist\XMST-0.1.0-alpha.exe` SHA256 `CED5315482D99532B51422FF9B091FDE01672F0DEEB1E6B1769B66BA7214282F`（15,316,480 B；cargo check 0 error；release 3m12s）。
  6. **待用户验证**：托盘→恢复→再关闭→再恢复 是否还有黑屏；双击 exe（已有托盘实例时）应**静默无提示**。

- 2026-10-02（第十二轮：磁盘清理 / 性能实测 / 字号预览精简 / 服务端平台识别与下载联动）：
  1. **磁盘占用分析（F:\XMST 原 10.18 GB）**：`target/` 8.84 GB（debug 4.97 + release 3.33 + GNU 目标 0.53）、`temp/` 1.33 GB（MinGW 工具链 1.28 GB + 我的截图）、`dist/` 176 MB、`backup/` 64 MB。**已清理**：`target/debug`、`target/x86_64-pc-windows-gnu`、`temp/` 下 MinGW 工具链与截图、`.research`、多余的 `dist/backup` → **当前 4.09 GB**（保留 `target/release` 以便增量构建）。⚠️ 构建工具链在 `C:\Users\xiumi\dev\tools\mingw64`（`.cargo/config.toml` 的 linker），**不是**被删的 temp 目录，删除后构建正常。
  2. **性能实测（工作集/私有字节/CPU）**：窗口显示默认 125 MB WS / 177 MB 私有 / CPU≈0；半透明 132–136 MB / 185–188 MB；亚克力 133 MB / 178 MB；**最小化 125 MB（不回收）**；**托盘（窗口隐藏）2.4 MB WS / 180 MB 私有 / CPU 0** —— 托盘内存**远低于用户预期的 30 MB**（Windows 隐藏窗口时自动裁剪工作集）。私有字节（提交大小）约 180 MB 是 GL 上下文+字体图集+Rust 堆的提交量，要显著下降需要隐藏时销毁窗口/GL 上下文（eframe 不易做到，暂不做）。
  3. **界面字号预览精简**：正文行由「正文示例 Aa 123 服务器状态」改为「正文示例」四字。
  4. **服务端平台识别（新模块 `src/serverinfo.rs`）**：从目录文件推断 **原版 / Fabric / Quilt / Forge / NeoForge / Paper / Spigot / CraftBukkit / Purpur / 混合端**，并给出 MC 版本与加载器版本，同时返回**识别依据**（evidence）。识别链：① `libraries/` 加载器目录（含 `fabric-loader/<ver>`、`minecraftforge/forge/<mc>-<forge>`、**`net/minecraft/server/<mcver>`**）；② 核心 jar 名（paper/spigot/purpur/mohist/magma/arclight…）并从名字抠版本；③ jar 内 `version.json`；④ **`logs/latest.log` 多格式匹配**（原版 `Starting minecraft server version X`、Fabric `Loading Minecraft X with Fabric Loader Y`、NeoForge、Forge `Forge Mod Loader version`）；⑤ `versions/<mcver>/`；⑥ `mods/` `plugins/` 计数。**实机验证目标**：`D:\Desktop\Fabric1.21.11` 确为 Fabric（`latest.log`: `Loading Minecraft 1.21.11 with Fabric Loader 0.19.5`、265 mods）—— 该格式是被本轮新增匹配覆盖的关键样本。
  5. **下载联动**：① 服务器-文件浏览-mods 页进入即识别平台，**原版/纯插件端把「下载模组」按钮置灰**并说明原因，模组端显示"已识别：Fabric · MC 1.21.1（加载器 fabric）"；② 点下载自动把识别到的**加载器与 MC 版本**写入下载页（`apply_platform_from_dir`）；③ 下载页"安装到"选择服务器时同样自动套用，并显示识别结果 + 「重新识别并套用」按钮（悬停看依据）；④ **概览页新增「🧩 服务端平台」行**（类型 · MC 版本 · 加载器版本 · 模组/插件数量，悬停看识别依据）。
  6. **构建注意**：一次 `cargo check` 曾因 `ring` 构建脚本在 `%TEMP%` 创建临时文件被拒而失败；把 `TMP`/`TEMP` 指到工作区内（如 `F:\XMST\temp\ctmp`）即可通过，release 构建不受影响。
  7. **产物**：`dist\XMST-0.1.0-alpha.exe` SHA256 `D7F2569555AAA45D048990B385A2845244F41E923FF82D0544700FFE378C9EB4`（15,315,968 B；cargo check 0 error；release 3m05s）。

- 2026-10-02（第十一轮：导航图标矢量重绘 / 标题栏徽标 / 日志卡片紧凑 / 删说明文本 / 折叠观感）：
  1. **左侧导航图标错位（用户第二轮反馈）真因**：图标是 emoji（📊 🗂 ⬇️ 🧩 🔗 ⚙️），来自**不同字体**（彩色 emoji 字体 vs 符号字体），字形宽度/基线/视觉重量各不相同，`Align2::CENTER_CENTER` 只能对齐"文本框"而非"墨迹" → 怎么居中都不齐。**修法**：新增 `App::nav_icon()`，六个图标**全部用矢量图元绘制**（柱状图 / 机架 / 下载箭头 / 插头 / 链条 / 推子）在固定 18×18 方框内居中 → 天生像素级对齐。截图确认 ✓
  2. **左上角 XMST 影响缩放**：原来「XMST」「v0.1alpha」是两个 `Label`（可选中文本），压在标题栏左上角会**吃掉鼠标按下事件** → 想拖左上角缩放变成"框选文字"。**修法**：品牌区改为 `painter` 直接绘制 —— 圆角强调色徽标（内嵌字母 X）+ 同行绘制的「XMST v0.1alpha」，**完全不产生交互控件**，事件全部归标题栏拖拽/边缘缩放。
  3. **设置-日志卡片仍偏大**：去掉两侧卡片 `set_min_height(300)`（改为 120 紧凑基线，高度由内容决定），"最近 5 条"滚动区 150 → **110**，卡片明显变矮。
  4. **删除通知说明文本**：按用户要求删除「服务器启动/关闭、隧道添加/停止/异常时…右下角仍可见」这句。
  5. **竖向折叠观感向隧道管理页对齐**：① 修掉一个真 bug —— 折叠箭头两个分支写的是同一个字形 `"▶"`（箭头永远不变 → 用户感觉"没有反馈"），现按 `anim` 在 **▼/▶** 间切换；② 折叠头部整行加**悬停底色**（`animate_bool_with_time` 0.12s），与隧道页行反馈一致；③ 动画期间**隐藏 ScrollArea 滚动条**（高度逐帧变化时滚动条会闪/抖，正是"不如隧道页丝滑"的来源）。
  6. **附带修复**：`apply_bg` 写"唯一真源"时**不再在 `Default` 状态写回** —— 之前停用插件/关闭效果会把用户选好的模式清成 `default`，下次启用效果"就没了"（本轮实测配置被清成 default 即此因）。
  7. **产物**：`dist\XMST-0.1.0-alpha.exe` SHA256 `F913A4199CFB9DB94F7FEB2282C4AA5FCB16EA2E9675AC6D27C908630ABCA5DB`（cargo check 0 error；release 3m04s；旧版保留 3 份备份）；内置截图确认导航六个矢量图标同尺寸同列对齐、左上角为绘制徽标。

- 2026-10-02（第十轮：8 项 UI/交互修复——导航对齐 / 缩放 / 闪烁 / 属性禁用 / 动效 / 字号预览 / 日志卡片 / 页边距）：
  1. **左侧导航图标错位**：图标改为画在 30px **单元格中心**（`Align2::CENTER_CENTER`），文字起点固定 `min.x + 44`，不再按左边缘对齐（不同 emoji 宽度差异是错位根因）；截图确认 ✓
  2. **窗口无法缩放**：① 热区加宽（左右下 9pt、上边 5pt 并补上「上边」方向）；② **普通状态改回系统 `BeginResize`**（命中测试/最小尺寸/跟手交给系统，"和正常窗口一样"；第九轮的自绘 `resize_drag` 仅保留给最大化时先还原再缩放）——**注意这是对第九轮"弃用 BeginResize"决定的反转**，原因是用户再次反馈缩放无效，若日后觉得系统边框交互突兀可改回；③ 标题栏**双击 = 最大化/还原**；④ 配置里被写坏的 `window_size`（2226×1252 = 最大化整屏）复位为默认，且 `window_is_maximized_now`（矩形 vs 工作区）继续防止再次写坏。
  3. **某些界面时不时闪一下**（改插件后出现）：① `apply_bg` 中 `SetWindowRgn` **只在模式真正变化时**重设（拖不透明度滑杆时每帧重设会周期性闪）；② 新增**纹理像素上限 4M**（ds=1 在 2560×1440 下 14MB/帧上传造成周期性长帧）；③ 背景配置落盘**去抖 600ms**。抓屏本身已在工作线程（第十轮前一轮完成）。
  4. **玩家属性暂时禁用**：新增 `ui_players_props_disabled`（只显示说明卡片），`PlayerTab::Props` 指向它；原实现 `ui_players_props` 保留备用。
  5. **平滑动效**：① 「设置页内平滑动画」并入「**切换动效**」总开关；② 侧栏折叠/展开改为宽度+文字**插值动画**（新字段 `nav_w_anim`，文字/分组标题/竖条按 t 淡入淡出）；③ 导航悬停底色用 `animate_bool_with_time`；④ 通用按钮走 `Style::animation_time`（开关控制 0.15s/0）。
  6. **界面字号实时预览**：滑杆右侧新增预览卡片，按 `theme::scaled_font` 同换算绘制正文/次要/小字/标题四行。
  7. **设置-日志卡片尺寸异常**：改 `horizontal_top`，右卡宽度自适应剩余空间（不再写死 300px），内层最近日志滚动区限高 150px。
  8. **下载/插件页文字贴左**：两页内容容器统一 +14px 左侧内边距。
  9. **产物**：`dist\XMST-0.1.0-alpha.exe` SHA256 `7F42D57A6B5D55C15A0AECAB142B444F4C111E5EBE5B64FBFBB130C3F0143BF5`（cargo check 0 error；release 2m59s；旧版备份保留 3 份）；部署配置：窗口几何复位、侧栏展开、动效开、插件启用、`bg_content_scrim=0.5`（沿用第八轮可读性结论）。
  10. **待用户实机验证**：导航对齐 / 边缘缩放与双击标题栏 / 插件页反复调参是否还闪 / 日志页卡片 / 下载插件页边距。

- 2026-10-02（第九轮：四问题处理——SmartScreen 警告说明 / 自绘自由缩放 / 特殊功能移入测试功能 / 折叠侧栏图标修正）：
  1. **背景**：用户四问——① 打开文件警告来源；② 无边框窗口无法像普通窗口自由缩放；③ 特殊功能应列入「测试功能」用开关控制显示；④ 折叠侧栏后图标显示歪斜。
  2. **① 打开文件警告**：截图确认系 Windows SmartScreen「无法验证发布者」安全警告（路径 `F:\XMST\dist\XMST-0.1.0-alpha.exe`）。未签名 exe 的正常安全警告，非程序缺陷，本轮不改代码；用户可点「更多信息 → 仍要运行」放行。
  3. **② 自绘自由缩放**：`main.rs` 新增 `resize_drag` 字段（方向+起始鼠标物理坐标+起始窗口物理矩形）与 `poll_resize_drag`；`handle_edge_resize` 弃用系统 `BeginResize`（会切系统边框交互），改为按住边缘 `GetCursorPos + SetCapture` 进入自绘缩放，逐帧按鼠标位移 `SetWindowPos` 调整窗口（东/南/西/北/四角），最小尺寸 960×600 逻辑像素按 DPI 换算，左键松开或窗口失效自动结束并 `ReleaseCapture`。`update()` 中 `handle_edge_resize` 后追加 `poll_resize_drag` 调用。
  4. **③ 特殊功能移入测试功能**：`features.rs` 中 `BETA_SPECIAL`（Spark 分析页总开关）`group` 改 `Beta`、`default_enabled=false`，注释改为「2026-10-02 移回测试功能，默认禁用」；`main.rs` 中 `ui_settings_beta` 的 `beta_list` 增加第 4 项 `(BETA_SPECIAL, "特殊功能（Spark 分析）")`，Nav::Special 与 ServerTab 特殊功能页注释同步；`dist\data\xmst_config.json` 中 `beta.server.special.enabled` 由 `true` 降为 `false`（默认关闭，可在测试功能开关中开启；子开关 spark 保持开启，总开关一开即可用）。
  5. **④ 折叠侧栏图标修正**：折叠态图标字号 14→16pt；选中态左侧竖条在折叠态下不再绘制（此前贴边竖条造成图标整体偏左的「歪斜」错觉，实际图标本身居中无歪斜）。
  6. **构建发布**：`cargo build --release` 3m07s 成功（74 warnings 历史遗留，含 GetAsyncKeyState i16 位与溢出已修）。旧版备份 `dist\backup\XMST-0.1.0-alpha_20261002_171519.exe`；新版 `dist\XMST-0.1.0-alpha.exe` 17:13:39 覆盖，SHA256 `16A44B4661285EAD8AD83404F25014DD17BA7C86F52D0AD752D9CBDA5D23D7D9`。
  7. **待用户实机验证**：自绘缩放拖动手感与最小尺寸、测试功能开关显示/隐藏特殊功能、折叠侧栏图标观感。

- 2026-10-02（第八轮：第七轮接续——重建发布 + theme.rs 编码损坏修复 + 三问题源码确认）：
  1. **背景**：第七轮遗留"源码与 dist 不一致（未重建）"，本轮接续完成重建发布。开工前发现 `src/theme.rs` 因 GBK 误转损坏：大量中文注释变乱码、若干行被注释吞并（`pub fn light_adapt`/`fn shade`/`pub fn auto_contrast`/`pub fn apply`/参数行/语句行），导致 cargo 编译失败。已按规范将乱码中文注释重写为英文注释，恢复被吞的代码行（清理 13 行残留乱码注释），`theme.rs` 修复后 cargo check 通过。
  2. **用户三问题源码确认（全部在位）**：① 拖动窗口闪烁 → `main.rs` 11515-11545：拖动/缩放中 `min_interval=3600s` 完全冻结抓屏，旧纹理 UV 偏移跟随，停止 160ms 沉降后立即补抓（`bg_needs_refresh`），2s 保险丝兜底；② 无边框窗口无法缩放 → `NativeOptions.viewport.with_resizable(true)` + `handle_edge_resize`（9px 热区 BeginResize）+ `window_is_maximized_now` 最大化跳过，窗口几何记忆由 `primary_screen_logical()` 钳制（修复巨大窗口）；③ 半透明不够清晰 → 内容衬底 `content_scrim` 默认 0.25 上调 0.5（serde default 与 `Default for GlobalConfig` 已统一），`load_config` 脏值自愈（<0.35 抬到 0.5 回写磁盘），材质模式内容面板有稳定衬底、文字可读。
  3. **构建发布**：修复 `theme.rs:181` 误加逗号后 `cargo build --release` 成功（3m17s，73 warnings 历史遗留）。旧版已备份至 `dist\backup\20261002_161013`。
  4. **产物（当前基线）**：`dist\XMST-0.1.0-alpha.exe` SHA256 `394F6FEEBAF9B89171DEA24D9A7741D3B4289DC62B24B6E20E31A54EB59FF8FA`（16:09:35，15,266,816 B，与 `target\release\XMST.exe` 一致）。第七轮"新会话接续点"至此完成。
  5. **待用户实机验证**：三问题（拖动闪烁 / 窗口缩放 / 半透明清晰度）真机复测；若半透明仍不够清晰，下一步方向：`content_scrim` 0.5→0.6 上限内再调、或背景档位 ds=2 保持、或衬底色向主题 bg 靠拢（当前为弱化白）。

- 2026-10-01（第七轮：插件功能更新收尾——源码微调批次未构建，待新会话接续）：
  1. **背景**：本轮为 Deepseek 对插件功能（毛玻璃/亚克力/半透明材质 + 插件卡片配置区）的更新收尾轮。第六轮产物 `dist\XMST-0.1.0-alpha.exe`（SHA256 `564B44F0...`，20:58 构建）部署后，用户实机运行至 21:23（`dist\data\bg_debug.log` 末条：`Translucent` 模式 opacity 滑杆调至 0.06–0.21、`accent_ok=false`、`captures=622`、`tex=1280x720`、`cap_ms=22.14/max47.33`——本机 DWM accent 仍不可用，桌面捕获式材质生效中）。
  2. **微调批次（已修复编译错误）**：20:58 构建之后 `src/backdrop.rs`（21:26:39）与 `src/main.rs`（21:27:13）又有修改，内容属桌面捕获式背景实现（抓屏+降采样+盒式模糊）微调；其中 **main.rs 有 2 处编译错误**：① 4360 行调用不存在的 `self.window_is_maximized()`（E0599），② 11494 行 `ds.saturating_mul(6)` 整数类型不明确（E0689）。本轮已修复——① 改为 `resolve_main_hwnd(&mut self.hwnd_cache)` + `window_is_maximized_now(hwnd, self.win_rect_px)`，② 解构元组加 `(i32, i32, bool)` 类型标注。**cargo check 已通过（7.43s，0 error，72 warnings 历史遗留）**，release 产物尚未重建，源码与 dist 仍不一致。
  3. **当前基线（务必以此为准）**：`dist\XMST-0.1.0-alpha.exe` SHA256 `564B44F078860023BC5D5A83A7B2787DFF489A5D0F9962FA1C2A75FCF6C00E0F`（20:58:10 与 `target\release\xmst.exe` 一致）；`dist\backup` 保留 20261001_040919 / 20261001_205317 / 20261001_205815 三份旧版。
  4. **新会话第一步（接续点）**：`cargo build --release`（check 已在第七轮通过，无需重跑）→ 覆盖 `dist\XMST-0.1.0-alpha.exe` → 核对 dist 与 release 的 LastWriteTime/SHA256 → 实机验证插件背景功能（三模式 + 插件卡片配置区 + 拖动/缩放冻结 + 自适应节奏）→ 回填本条目为已验证。

- 2026-10-01（第六轮：亮色看不清 / 半透明不清晰 / 启动告警 / 侧栏折叠）：
  1. **用户反馈**：① 亮色模式看不清；② 半透明背景也糊；③ 启动多一个"打开警告"；④ 部分字体（如左栏「仪表盘」）不跟随变色；⑤ 左栏要能折叠成纯图标（左下角 ◀/▶）。
  2. **①②④ 共同根因 + 修法**：`fg()` 与 `theme_is_light()` 读的是**配置主题**而非**材质翻转后的实际调色板** → 界面已变浅、导航/图标仍返回深色主题的浅灰浅青。现以 `theme_cur.text` 亮度判定；左栏颜色取调色板（选中=accent、普通=weak）；`auto_contrast` 同时适配强调色。**半透明**由 `ds=4+模糊半径1` 改为 **`ds=2` 且不模糊**（纹理 369×238 → 1031×664，实机截图可直接读清背后桌面文字）；单次抓屏升到 ~31ms，故加**自适应节奏**（内容在变 120ms / 静止逐步放宽到 600ms）。
  3. **③ 启动告警**：背景恢复弹出的提示条（含「DWM 不可用已回退」字样）每次开机都出现 → 启动恢复与插件脚本（`on_enabled`）触发改为**静默**（新字段 `bg_suppress_toast`），仅手动切换才提示，并改友好措辞。
  4. **⑤ 左栏折叠**：新配置 `nav_collapsed`；折叠后 56px 只显示图标（居中、整行可点击切页），隐藏分组标题与保存按钮；左下角 ◀/▶ 按钮（`bottom_up` 布局）切换并持久化。截图确认 ✓
  5. **产物**：`dist\XMST-0.1.0-alpha.exe` SHA256 `564B44F078860023BC5D5A83A7B2787DFF489A5D0F9962FA1C2A75FCF6C00E0F`（cargo check 0 error；release 3m09s；旧版保留 3 份）。

- 2026-10-01（第五轮：修窗口记忆巨大化 + 拖动实时更新 + D4 单一真源）：
  1. **用户反馈**：「默认窗口大小变得好大」「只有松开拖拽时才更新背景」。
  2. **窗口巨大化根因**（F1 引入）：把 `GetWindowRect` 的**外框物理矩形 / ppp** 当成 `window_size` 存储 —— ① 外框含不可见缩放边框 → 每次重启都变大（棘轮）；② **最大化时外框 = 整屏** → 下次启动开出巨大窗口。**修法**：改用 egui **客户区逻辑尺寸** `ctx.screen_rect().size()`（即 `with_inner_size` 语义）；**最大化时不记录**；还原时按主显示器尺寸钳制 + 位置越界校验；新增设置页「重置窗口位置与大小」按钮；配置里被写坏的 `window_size` 已复位。实测：外框 1180×760；运行后记忆 `[1283,826]`（客户区逻辑，ppp1.15 对应 1475×950 物理，不再棘轮）。
  3. **拖动实时性**：上一轮为不掉帧改成「拖动期间完全冻结」→ 松手才更新。现改为：拖动中**仍每 ~200ms 抓一次**（10~25ms 摊到多帧）；两次抓取之间用 **UV 偏移**按窗口位移平移底图（**零成本**、视觉连续跟随）；拖动结束那一帧 `min_interval=0` 强制补抓；保留 2s 保险丝。`backdrop::update()` 的 `force: bool` 形参改为 `min_interval: Duration`。实测模拟拖动期间 `captures` 3→20 ✅
  4. **D4 单一真源（现场证据）**：`plugin_configs[...].bg_alpha="0.32"`（启动恢复读它）与 `plugin_bg_alpha=0.55`（UI 滑杆写）并存 → 用户调的值重启后失效。现 `apply_bg()` 只写插件配置一处并同步历史字段（新增 `App::MATERIAL_OWNER_FALLBACK`），实测启动值稳定 0.70。
  5. **产物**：`dist\XMST-0.1.0-alpha.exe` SHA256 `F69AE157F4F9E589D117A9F5216013985347E79373DA630473AEED8828E626E3`（cargo check 0 error；release 3m08s；旧版保留 3 份备份）。

- 2026-10-01（第四轮：修「底图永不更新」+ F1/F2/D3 剩余功能）：
  1. **用户反馈**：「背景永远不会更新，一直维持着刚打开的背景」。
  2. **根因（上一轮修拖拽掉帧时自造）**：冻结判据用了「当前矩形 vs **上次抓取时**矩形」，而 `last_rect` 只在真正抓取时更新、冻结分支又不抓取 → 一旦几何变化过（启动时 ppp 由 1.25 变 1.15 就会触发）判据永远为真 → **底图永久冻死在首帧**。
  3. **修法**：判据改为「本帧 vs **上一帧**矩形」（是否正在移动）+ 停止后 160ms 沉降窗口；冻结结束后由 `update()` 内部的几何判据自动补抓；新增**保险丝：冻结绝不超过 2 秒**（同类 bug 结构上不可能再发生）。
  4. **实证**：脚本移动窗口（+300/+80）后再 `SC_MAXIMIZE`，日志 `captures` 由 2→3（移动前）→**20**（移动后）→**40**（最大化后，纹理 369x238 → 645x352 按新几何重建）✅
  5. **顺带修复**：① 内容衬底被 UI 滑杆钳制成 `0.0` → 复位 0.25；② 插件 `bg_alpha` 被测试拖到 `0.05`（日志 `opacity=0.05` + 明亮桌面 → 界面整体发白，与用户上轮「偏白」反馈吻合）→ 复位 0.70。
  6. **新增功能**：**F1** 窗口位置/大小记忆（`window_pos`/`window_size`，启动还原 + 越界校验，几何变化后延迟 800ms 落盘）；**F2** 效果可用性自检（`plugins::material_env_warnings()` 读 `EnableTransparency`/`SM_REMOTESESSION`/`GetSystemPowerStatus().Reserved1`，插件页显示 ⚠——TranslucentTB 完全缺失这一环）；**D3** 插件权限模型（manifest `permissions`，**留空=兼容模式全授予并记提醒**，示范包 v1.2.0 已声明三项）。
  7. **产物**：`dist\XMST-0.1.0-alpha.exe` SHA256 `04E12E3AE9F8CDFEDE72EEBCA267390BCC104CE4713FE9EF6F234EA2F090F883`（cargo check 0 error；release 3m08s）；插件 zip 两处同步重建（1344 B，v1.2.0，UTF-8 无 BOM）。

- 2026-09-30（第三轮：对标 TranslucentTB + 文字可读性 + 亚克力拖拽掉帧）：
  1. **TranslucentTB 源码核对结论**（分支 release@d4636e4）：它没有 blur.cpp，Win10 路径就是 `SetWindowCompositionAttribute(WCA_ACCENT_POLICY)`（我们已试过的第 3 种），`AccentFlags` acrylic=0 / 其余=2，ABGR 渐变、acrylic 有 `A==0→A=1` 钳制；不使用 `DwmEnableBlurBehindWindow`/`DWMWA_SYSTEMBACKDROP_TYPE`；**关键差别是它把 accent 下发给 explorer 的 `Shell_TrayWnd`——那窗口不画客户区背景**，而我们的 GL swapchain 每像素都画，accent 被完全遮住。→ 无可借鉴的第五种机制；已借鉴其 `AccentFlags` 语义（毛玻璃 state 3 改用 flags=2）。
  2. **文字可读性**（Fluent/WinUI/WCAG/Windows Terminal 研究结论：本质是不透明度/方差问题）：① 新增「内容衬底」滑杆（默认 0.25，即 WinUI content-layer 思路）；② 材质模式配色改近黑/近白极值 + 控件色 `shade(材质, ±Δ)`，合成色含衬底；③ **全局 1 物理像素文字描边**（≤16pt 文字复制 4 份反向明暗偏移副本，WCAG 2.2 SC 1.4.3 Note 5 明确「贴住笔画的窄边框算文字本身」）。描边通过 `[patch.crates-io] eframe = { path = "vendor/eframe-0.29.1" }` + 在 `glow_integration.rs` tessellate 前展开 `Shape::Text` 实现——**顺带把此前一直是死代码的「托盘隐藏不 Poll 空转」P0 补丁真正接入二进制**（构建日志确认 `Compiling eframe v0.29.1 (F:\XMST\vendor\eframe-0.29.1)`）。
  3. **亚克力拖拽掉帧根因（实测）**：单次抓屏 17~30ms（HALFTONE 对 1475×950 源做高质量滤波），而 60fps 一帧 16.7ms，拖动时又每帧重抓 → 必然掉帧。修法：`StretchBlt(COLORONCOLOR)` 点采样 + 小图盒式模糊补质量（稳态 ~10ms）、**拖动/缩放期间冻结底图**（几何变化不抓，停止 ~140ms 后补抓，拖动零抓屏开销）、复用 GDI 内存 DC/位图、静止间隔 80→120ms；日志新增 `cap_ms/max`。
  4. **验证**：cargo check 0 error（71 warnings）；release 3m01s；`dist\XMST-0.1.0-alpha.exe` 15,257,600 B 08:40，SHA256 `58B4C25EB9126ED2F50E47A3FF042435950551BD85C02B33521F2A8180A952C3`（与 release 一致）；内置截图确认 UI/文字（含描边）/材质/噪点均正常渲染。
  5. **文件**：src/backdrop.rs（COLORONCOLOR+盒式模糊+GDI 复用+cap_ms）、src/main.rs（模式分档 ds/blur/噪点、拖动冻结、内容衬底与开关、自动配色）、src/theme.rs（衬底/极值配色）、src/plugins.rs（AccentFlags 分状态）、src/config.rs（bg_content_scrim 等）、vendor/eframe-0.29.1（文字描边补丁）、Cargo.toml（[patch.crates-io]）、docs/背景材质失效根因分析_毛玻璃亚克力半透明.md（§10.9）。
  6. **给用户的判断依据**：若系统自身（开始菜单/设置）的亚克力也是纯色不透明，则本机 DWM 亚克力在驱动/虚拟显示栈层面失效，宿主代码无法修复，桌面捕获式材质即最终答案。

- 2026-09-30（用户复测第二轮小问题：毛玻璃/亚克力观感相同、低浓度界面偏白）：
  1. **毛玻璃 vs 亚克力无区别的根因**：本机 DWM accent 不可用（§10.2 已证），而材质阶段两者此前共用同一档降采样（12×）→ 唯一语义差别（模糊 vs 模糊+噪点）从未被画出来。**修复**：按模式分档 —— 半透明 3×、毛玻璃 9×、亚克力 **20× + 噪点层**（96×96 重复寻址纹理 `TextureWrapMode::Repeat`，alpha 16，懒创建）；真机实测中央区域相邻像素差：半透明 1.33/1.04、毛玻璃 1.15/0.80、亚克力 **6.31/6.31**（噪点高频能量显著）。
  2. **「不透明度调低背景偏白」的解释**：① 物理上材质 = 主题底色×浓度 + 桌面底图×(1−浓度)，浓度低则明亮桌面占比高；② `theme::auto_contrast` 在材质 luma>140 时整体切浅色界面以保证「浅字+亮底」可读，这才是界面偏白的直接原因（刻意适配）。
  3. **新增开关**：`GlobalConfig.bg_material_auto_contrast`（默认 true）+ 插件页复选框「界面配色跟随材质明暗」。关闭后固定主题配色，材质浓度自动抬到 **0.60 下限**（否则明亮桌面下不可读）——即「深色 UI」与「极低浓度」不可兼得，按可读性优先取舍（`App::effective_bg_opacity()`）。
  4. **验证**：cargo check 0 error（71 warnings）；release 3m00s；`dist\XMST-0.1.0-alpha.exe` 15,253,504 B 07:48，SHA256 `D770CCA409B69C468F25D3CE2881D1562F563EF26B9A2385506644AEB80117EA`（与 release 一致，旧版按 3 份保留）；部署配置复位为半透明 / 0.70 / 插件启用 / 两个新开关均为 true。
  5. **文件**：src/main.rs（模式分档、噪点纹理、`effective_bg_opacity`、复选框）、src/theme.rs（`auto_contrast_material` 形参）、src/config.rs（新字段）、docs/背景材质失效根因分析_毛玻璃亚克力半透明.md（§10.7）、docs/工具缺陷盘点与优化建议_2026-09-30.md、HANDOVER.md。

- 2026-09-30（材质盖住 UI 的根因修复 + 内置 F12 截图，附实测对照）：
  1. **用户反馈**：「底色透明了，但按钮/UI 完全看不见」。
  2. **取证手段**：新增内置自测截图（`XMST_SHOT=<路径>` 从**自己的帧缓冲**取图，不受 `WDA_EXCLUDEFROMCAPTURE` 影响）→ 得到硬数据：默认模式帧缓冲 `maxLuma=238 / 亮度>120 像素 619`；半透明 `maxLuma=68 / 0`；毛玻璃 `maxLuma=75 / 0` —— **UI 根本没被画出来**。
  3. **根因**：egui 的 `CentralPanel` 内容画在 `LayerId::background()` 层（`egui-0.29.1/src/context.rs:538-548` 注册 background area），同层内按插入顺序绘制；而 `paint_bg` 是在 `update()` **帧末**往该层追加材质 → 材质压在中央内容之上。这也是历史反馈「背景图盖住按钮」的同源问题（当时只把侧栏改不透明绕开）。
  4. **修复**：`paint_bg(ctx)` 从帧末移到 **`tick_theme` 之后、任何面板之前**（`main.rs::update` 开头）。修复后实测：半透明 `max=238 / 742`、毛玻璃 `max=238 / 752`，UI 全部恢复；材质模式下前景色仍由 `theme::auto_contrast` 按材质合成色自适应（明亮材质→深字，深色材质→浅字），控件底色用 `shade(材质, ±delta)` 保证有对比。
  5. **新增 F12 内置截图**：效果开启时本窗口被排除在系统截屏之外，外部截屏/录屏/PrintWindow 均拍不到；F12 从自己的帧缓冲取图存 `data\screenshot_<时间>.bmp`（自写 BMP 头——改用 PNG 编码器会让 exe 从 15.2MB 涨到 18.7MB，已避免）。
  6. **验证**：cargo check 0 error（71 warnings）；release 3m14s；`dist\XMST-0.1.0-alpha.exe` 15,251,968 B 02:07，SHA256 `1DFDD2443DD78BBF5901E8F6397BB6FB4B51BB9B487C7FFABD542DDE68364A9C`（与 release 一致，旧版按保留 3 份备份）；部署配置复位为半透明 / 不透明度 0.70 / 插件启用。
  7. **文件**：src/main.rs、src/theme.rs、docs/背景材质失效根因分析_毛玻璃亚克力半透明.md（§10.4-10.5）、docs/工具缺陷盘点与优化建议_2026-09-30.md（已修-11/12）、HANDOVER.md。

- 2026-09-30（材质可读性修复：用户实测反馈「只有一块模糊玻璃，按钮/UI 完全看不见」）：
  1. **根因**：第一版捕获方案只把底图铺在 background 层、面板仍按滑杆做半透明填充 → 深色主题的**浅色文字落到明亮桌面上**（浅字+亮底 ≈ 2:1 对比度），视觉上等于「UI 消失」。
  2. **修法**：① 材质 = 捕获底图 + 主题底色按滑杆不透明度着色，统一画在 background 层（`paint_bg`）；② 材质模式下面板填充 alpha 归 0，不再叠第二层；③ 新增 `theme::auto_contrast()`，`tick_theme` 用「材质合成色」调用它，自动选深/浅文字/控件/描边（明亮材质→深字，深色材质→浅字）；④ 顶栏与侧栏/弹窗改用 `window_fill` 的「材质近不透明版」（alpha 236），与内容区同色系；⑤ 材质模式下不再叠加用户壁纸。
  3. **默认值**：不透明度默认 0.55 → 0.70（纯白桌面下材质 luma ≈ 89，浅字对比度 ≈ 5:1）。
  4. **验证**：cargo check 0 error（71 warnings）；release 构建 3m26s；真机冒烟：`opacity=0.70`、`captures=1 tex=369x238`、捕获均值 `(34,34,34)`；`dist\XMST-0.1.0-alpha.exe` 15,248,384 B 01:29，SHA256 `D92EE432D4F44B6F1D0DFD0C71360D8C856E320DDFBCB0A221AA268D5AAB39A8`。
  5. **文件**：src/theme.rs、src/main.rs、docs/背景材质失效根因分析_毛玻璃亚克力半透明.md（§10.4）、docs/工具缺陷盘点与优化建议_2026-09-30.md（已修-11）、HANDOVER.md。
  6. **待用户复测**：三模式下 UI/按钮是否清晰可读、切换模式与拖动滑杆的观感、圆角是否正常。

- 2026-09-30（第二轮收尾：修复已知缺陷 D1/D2/D5/D6/D7/D9/D11 + 抓屏排除开关，构建部署待用户实测）：
  1. **D1 主窗口 HWND 缓存**：新增 `resolve_main_hwnd()`（`IsWindow` 校验 + 缓存），替换 `apply_window_round_region`/`apply_bg`/`paint_bg` 里的每帧 `main_hwnd()` 全系统 `EnumWindows`；`main_hwnd()` 仍保留给托盘等冷路径。
  2. **D2 背景效果归属仲裁**：新增 `App.plugin_bg_owner`，`apply_bg(..., owner)` 记录请求方；插件页**禁用/卸载持有效果的插件时自动恢复默认背景**并提示（以前插件停了窗口还透着）。
  3. **D5 就绪判定 + 兜底**：`server_started` 的判定从 `buf.contains("Done (")` 放宽为 `"Done (" + "s)"`；并新增超时兜底——180s 未出现就绪标志时，除提示外**补发一次 `server_started`**，避免非标准/代理端插件永远不触发。
  4. **D6 路径安全**：新增 `plugins::cache_key()`（滑杆化为 `[A-Za-z0-9._-]`、截断 64 字符、空/`.`/`..` 兜底），插件缓存目录与 `xmst_resource_dir()` 统一使用。
  5. **D7 日志轮转**：`bg_debug.log` 超 256KB 保留最后 200 行；`crash.log` 超 1MB 滚动为 `crash.log.1`。
  6. **D9 事件单播**：新增 `PluginManager::emit_to(name, event, args)`；`enabled` 事件改为单播（以前广播会让所有已启用插件各重设一次背景）。
  7. **D11 抓屏排除开关**：`GlobalConfig.bg_capture_exclusion`（默认 true）+ 插件卡片内复选框 + 说明文案；关闭时不抓桌面（避免自我递归），效果退化为面板半透明。同时修正卡片提示文案为「半透明=窗口背后桌面透进来；毛玻璃/亚克力=同一张底图重度模糊」。
  8. **验证**：cargo check 0 error（71 warnings 历史遗留）；cargo build --release Finished 4m06s；真机冒烟：进程启动正常、`bg_debug.log` 出现 `captures=1 tex=369x238` 与真实桌面样本、`plugin_states` 保持 `xmst-frosted-glass-demo=true`；`dist\XMST-0.1.0-alpha.exe` 15,246,848 B 01:14，SHA256 `1409013853FDA1BF3635EDFC07763FC4CD3F77E8D12C2821A8C5B2F3576C7AE0`（旧版备份保留 3 份）。
  9. **文件**：src/main.rs、src/plugins.rs、src/config.rs、docs/工具缺陷盘点与优化建议_2026-09-30.md、HANDOVER.md。
  10. **待用户实测**：三模式 + 不透明度滑杆、圆角是否保持、禁用插件是否自动恢复背景、重启是否保持启用与效果。

- 2026-09-30（背景材质第二轮：真机实测确认本机所有系统级透明机制失效 → 改为「桌面捕获式背景」，三模式全部可用）：
  1. **实测结论（关键，避免后人重复踩坑）**：本机 Win10 19045 + GTX1060 + 虚拟/远程显示适配器（GameViewer/Parsec/AskLink）环境下，逐像素窗口 alpha（`DwmEnableBlurBehindWindow` 空区域/整窗区域）、`DwmExtendFrameIntoClientArea(-1)`、`SetWindowCompositionAttribute` 亚克力、`WS_EX_LAYERED`+`LWA_ALPHA` **全部调用成功但像素零变化**（对照表见 docs\背景材质失效根因分析_毛玻璃亚克力半透明.md §10.2）。根因是 OpenGL 呈现路径不透明（DXGI flip 模型/远程会话），宿主程序无法改变。
  2. **替代实现 `src/backdrop.rs`（桌面捕获式背景）**：`SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` 排除本窗口 → GDI `BitBlt` 抓窗口所在屏幕区域 → `GetDIBits` 读 BGRA → 盒式降采样（半透明 4×、毛玻璃/亚克力 12×）→ 作为 egui 纹理画在 background 层，面板按 `plugin_bg_opacity` 叠加；80ms 节流 + 窗口几何变化强制重抓；退出效果时撤销抓屏排除。
  3. **踩坑登记（务必记住）**：`GetDIBits` 要求位图**未选入 DC**，否则直接失败且无任何错误码（表现为 `captures=0`，本轮实测踩坑）——必须先 `SelectObject` 取消选择再调用；`SetLayeredWindowAttributes` 对 GPU 呈现的 OpenGL 内容完全无效（只作用于 GDI 重定向表面）。
  4. **验证（真机 + 探针）**：`bg_debug.log` 记录 `backdrop mean/samples/captures/tex`；自捕获判别测试（把本工具主题临时改成纯白后重跑）捕获样本仍为桌面内容 `(21,21,23)` 而非 `≈240`，证明抓的是窗口背后的真实桌面、无自我递归。
  5. **同期修复（第一轮遗留）**：三模式独立（`BgStyle` 四态）+ 各自可调不透明度、不透明度 0 的历史陷阱归一化、accent 返回值不再丢弃（三级回退）、accent 后强制重设 `SetWindowRgn`（圆角不再变尖锐直角）、插件启用状态落盘、`start_server` 复位 `plugin_start_emitted`、新增 `on_enabled` 事件、启动恢复背景效果。
  6. **文件**：src/backdrop.rs（新增）、src/plugins.rs、src/theme.rs、src/main.rs、data/plugins 与 dist/data/plugins 的 xmst-frosted-glass-demo.zip（1.1.0）、docs/背景材质失效根因分析_毛玻璃亚克力半透明.md（新增 §10）、docs/工具缺陷盘点与优化建议_2026-09-30.md（新增）、HANDOVER.md。
  7. **构建与产物**：cargo check 0 error（71 warnings 历史遗留）；cargo build --release Finished 4m23s；release 产物 15,2xx,xxx 字节 00:58，SHA256 `862725096796621A62C3357040791CFA92E75000911C44B1FF9CDCAC01A08CE5`，`dist\XMST-0.1.0-alpha.exe` 已覆盖且哈希一致（旧版按保留 3 份策略备份于 dist\backup\）。
  8. **副作用告知**：效果开启期间 XMST 会从截屏/录屏/共享屏幕中消失（抓屏底图不自我递归的前提）；已在缺陷盘点文档 D11 列为待办（加配置开关 + UI 说明）。

- 2026-09-29（插件背景材质彻底修复：三模式独立 + 各自可调不透明度 + 启用状态持久化 + 圆角被 accent 重置的根因修复，cargo check 0 error（71 warnings 历史遗留），cargo build --release 3m29s/3m2x Finished）：
  1. **用户实测三条线索**：① 插件每次重进都变回关闭、必须重新启用；② 选毛玻璃时圆角变成「半透明尖锐直角」，中心基本不变，不透明度滑杆只影响那一小块；③ 半透明模式完全没效果、角落全透。据此定位到 4 个必现缺陷（详见 docs\背景材质失效根因分析_毛玻璃亚克力半透明.md §9）。
  2. **插件触发链修复（对应分析 A1/A2/A3）**：插件页「启用/禁用」「卸载」后把 `PluginManager.states` 写回 `cfg.plugin_states` 并 `save_config()`（修复重启丢失启用状态）；`start_server` 复位 `rt.plugin_start_emitted`（此前 `ServerRuntime` 每服只建一次、该门控从不复位 → `server_started` 每次程序运行只发一次）；新增宿主广播事件 `enabled`（脚本钩子 `on_enabled`），点「启用」立即应用效果；`tick_plugins` 首次 tick 按已启用插件保存的 `bg_style/bg_alpha` 恢复背景效果；关闭 BETA_PLUGINS 总开关时自动复位背景。
  3. **三模式 + 独立不透明度（对应分析 B1/B2）**：`plugins::BgStyle` 由 3 态改为 4 态 `Default/Translucent/Frosted/Acrylic`（毛玻璃=DWM `ACCENT_ENABLE_BLURBEHIND`(3)，亚克力=`ACCENT_ENABLE_ACRYLICBLURBEHIND`(4)，两者视觉不同、各自独立）；`main.rs` 的 `plugin_bg_active: bool` 换成 `plugin_bg_style + plugin_bg_opacity: f32`；`theme::apply` 中半透明模式面板 alpha = 该插件不透明度（in-app 蒙层，**不再依赖背景图开关**），毛玻璃/亚克力 = 0（透出 DWM 模糊）；卡片配置区改为三模式按钮 + 通用「不透明度」滑杆（0.05-1.0），滑杆对三种模式即时生效。
  4. **亚克力与圆角（对应分析 B4 与用户线索②）**：DWM 着色由「纯黑 + 全局暗度」改为「主题底色 + 该插件不透明度」，启用时 alpha 最小 1（社区实证 acrylic 在 alpha=0 时不出模糊）；`apply_accent` 返回值不再丢弃，按「亚克力→毛玻璃→半透明」三级回退并 toast 说明实际模式；**圆角变尖锐直角的根因** = DWM accent 会重置窗口区域（SetWindowRgn 结果被抹掉），而 `apply_window_round_region` 有 `(r_px,w,h)` 去重缓存 → 圆角永久丢失，修复为应用 accent 后失效缓存并在同一步用当前半径重设 SetWindowRgn（新增 `win_r_points` 字段）。
  5. **历史 0 暗度自愈**：旧「毛玻璃暗度」滑杆拖到 0 会写 `plugin_bg_alpha=0.0`（用户配置正是 0.0，等于「点了没反应」），新增 `plugin_opacity_default()`：< 0.05 一律取 0.55，并写回插件 `bg_alpha`，保证滑杆显示值与实际浓度一致。
  6. **运行时诊断**：新增 `data\bg_debug.log`（`requested/effective/opacity/accent_ok/hwnd_null/layered/composition/gl_alpha_bits/ppp`）——DWM accent 是未公开 API，失败时无返回值无日志，eframe/winit 的 log 在本程序未初始化，这是唯一可靠的定位手段。
  7. **插件包**：`xmst-frosted-glass-demo.zip` 升 1.1.0（新增 `on_enabled`，样式键 `translucent/frosted/acrylic/default`），按「必须重建而非增量改」的既有教训重建，`data\plugins` 与 `dist\data\plugins` 两处一致。
  8. **文件**：src/plugins.rs、src/theme.rs、src/main.rs、data/plugins/xmst-frosted-glass-demo.zip、dist/data/plugins/xmst-frosted-glass-demo.zip、docs/背景材质失效根因分析_毛玻璃亚克力半透明.md、HANDOVER.md。
  9. **验证**：cargo check 0 error（71 warnings）；cargo build --release Finished 3m23s；release 产物 15,229,440 字节 13:45:41，SHA256 `2AFBB29A07098ACB665E5FDD4A973ACEA942313EECE423FAB90BE688895A29A2`，`dist\XMST-0.1.0-alpha.exe` 已覆盖且哈希一致（旧版依次备份 20260929_133708 / 134152 / 134545，保留最近 3 份）；`dist\data\xmst_config.json` 预置 `plugin_states.xmst-frosted-glass-demo = true` 与 `bg_alpha = 0.55`（原为历史遗留的 0.00，会表现为「点了没反应」）；真机验证方式见分析文档 §9.5。

- 2026-09-29（Defender 功能排查结论 + 用户决定跳过不修复，本轮收尾：会话总结至交接文档、清理临时文件、release 验证构建）：
  1. **Defender 故障现象**：Windows 安全中心打开报「文件系统错误 (-2147219195)」（拒绝访问/权限类）；用户反馈 Defender 一直没生效（无任何防护记录/历史），怀疑第三方优化工具剥离。
  2. **诊断结论（三因叠加，本机无法修复验证）**：① 组策略锁死——HKLM\SOFTWARE\Policies\Microsoft\Windows Defender 下 DisableAntiSpyware / DisableAntiVirus / DisableRealtimeMonitoring / DisableAntiSpywareRealtimeMonitoring / DisableAntiVirusRealtimeMonitoring 等 Disable* 策略键为 1；② SecurityHealthService 服务被设为 Disabled（Windows 安全中心依赖，禁用即打不开）；③ WinDefend 服务与平台组件缺失（服务注册项/目录被删，DISM 联机修复因组件源也缺失不可行）。用户自认曾手动删除 Defender 组件。
  3. **用户决策**：跳过 Defender 功能修复（本机为唯一开发/使用机且无第二台正常电脑可对照）。XMST 设置页 Defender 区块（排除数据目录 / 完全禁用实时保护）的本机真实效果验证无法进行，**此项作为已知限制长期保留**。
  4. **验证方案存档（供有正常电脑后执行）**：路线 A Windows Sandbox（本机 Win10 专业版 19045 可开，需 BIOS 开 SVM + 启用「Windows 沙盒」功能）；路线 B VirtualBox + Win10 评估版虚拟机；路线 C 命令层验证——提权流程、命令拼装、错误回传是否正常 + EICAR 测试字符串清单（排除目录放 EICAR 不报毒 / 未排除目录报毒 / Get-MpPreference 核对排除项写入）。验证要点：篡改保护需先关、Defender 恢复后需清理策略键与 Services 注册残留。
  5. **本轮收尾（验证通过）**：会话总结已写入本文档（本条）；临时日志等中间产物已清理；release 验证构建 Finished（0 error，71 warnings 均为历史遗留），target\release\xmst.exe 与 dist\XMST-0.1.0-alpha.exe SHA256 一致 D4B50282F28186406AAD57371D12365827ABD845DCA9AE7162C152F7E265791F（15,219,712 字节，09:01:05），交接无编译问题。
- 2026-09-29（修复毛玻璃仍只圆角生效 + 系统日志覆盖右上角窗口按钮，cargo build --release 后台 3m31s Finished（71 warnings 0 error，均为历史遗留），release 产物 15,219,712 字节，SHA256 D4B50282F28186406AAD57371D12365827ABD845DCA9AE7162C152F7E265791F，dist\XMST-0.1.0-alpha.exe 待用户关闭运行中进程后覆盖，旧版 dist\backup\20260929_XXXXXX\ 待覆盖前生成）：
  1. **毛玻璃仍只圆角生效根因**：窗口并非真透明——eframe NativeOptions 未 with_transparent(true)，运行时 ViewportCommand::Transparent(true) 在 winit/Windows 下不可靠（仅置标志/加 WS_EX_LAYERED，不建立 alpha surface），DWM 亚克力无法与 egui 自绘内容混合；egui 不透明像素盖住亚克力层，仅 SetWindowRgn 圆角裁剪边界露出模糊带 → 视觉表现为"只有圆角角落生效"。修复：NativeOptions 增加 .with_transparent(true)（创建期透明底座，wgpu surface 带 alpha）；App 实现 clear_color——plugin_bg_active 时返回 [0,0,0,0] 全透明（毛玻璃/半透明整窗透出），否则不透明 (12,12,12) 兜底；apply_bg 删除全部运行时 ctx.send_viewport_cmd(Transparent)（WS_EX_LAYERED 破坏 DWM 合成），Translucent 分支先 apply_acrylic(hwnd,false,0) 清除亚克力；theme.rs 中央 panel_fill alpha=0 逻辑（上轮）继续生效；结构性面板（SidePanel）保持不透明防遮按键。
  2. **日志覆盖右上角按钮根因**：标题栏中区 toast（系统日志，[HH:MM:SS] 前缀）与服务器名两个 Label 未截断，长文本画出子 Ui 可用区（egui 不自动裁剪），溢出覆盖右侧最小化/最大化/关闭按钮。修复：两处改为 ui.add(Label::new(...).truncate())，按可用宽度截断加省略号。
  3. **文件**：src/main.rs（NativeOptions/clear_color/apply_bg/titlebar 两 label）、src/plugins.rs（apply_acrylic 注释更新）。
  4. **验证**：cargo check 0 error；cargo build --release 后台 3m31s Finished；release 产物 target\release\xmst.exe 15,219,712B 09:01:05；dist\XMST-0.1.0-alpha.exe 被运行中进程（PID 16944，08:28 启动）锁定，待用户关闭后覆盖并核 SHA256；误按旧双 exe 策略复制的 dist\xmst.exe 与 dist\修暝的服务器工具.exe 已删除（回收站）。
  5. **交付完成**：用户授权后确认占用进程已退出，旧版备份 dist\backup\20260929_125852\；dist\XMST-0.1.0-alpha.exe 已覆盖为 9:01 release 产物，SHA256 D4B50282F28186406AAD57371D12365827ABD845DCA9AE7162C152F7E265791F 与 release 一致；按保留 3 份策略清理双 exe 时代最旧归档 dist\backup\20260929_074009\（回收站）。
- 2026-09-29（修复 xmst-frosted-glass-demo.zip 损坏致「未发现有效插件」：上一轮用 .NET ZipArchive Update 增量改 main.rhai + PowerShell Get-Content 按 ANSI 解码 UTF-8，导致 zip 结构异常且脚本中文乱码（826B → 877B），Rust zip crate 解析/编译失败，reload_all 后 ok=0 报「未发现有效插件」。已用 .NET Create 模式按原始字节重建 zip（manifest.json 357B + main.rhai 826B，UTF-8 无 BOM，中文正常），data\plugins 与 dist\data\plugins 两处一致；无 XMST 进程在跑，重启程序或点「重新扫描」即可加载）：
  1. **踩坑登记**：改插件 zip 必须重建而非增量 Update；PowerShell 读 UTF-8 文件用 -Encoding UTF8 或直接按字节复制，禁止 Get-Content 默认 ANSI 解码后写回。
  2. **文件**：data/plugins/xmst-frosted-glass-demo.zip、dist/data/plugins/xmst-frosted-glass-demo.zip。
- 2026-09-29（毛玻璃仅圆角角落有效根因修复 + 背景效果入口移入单插件卡片「配置」区，cargo check 0 error（71 warnings 历史遗留，本次改动零新增 warning），cargo build --release 3m23s Finished，dist\XMST-0.1.0-alpha.exe 已覆盖 08:06:39，SHA256 471E65556D8C1AB1EBE26E2DF55E7272D7C50626BD3404FBC70A38FA3B08A06E，旧版备份 dist\backup\20260929_080815\）：
  1. **毛玻璃只角落生效根因**：apply_acrylic 走 DWM 亚克力（SetWindowCompositionAttribute ACCENT_ENABLE_ACRYLICBLURBEHIND），需窗口有真实透明区域；theme.rs apply 中 panel_alpha 未开背景图时恒 255（不透明 egui 填充全覆盖亚克力），仅 SetWindowRgn 圆角裁剪边界露出部分亚克力 → 视觉表现为"只有圆角角落那一小块有效"。修复：theme.rs apply() 新增 acrylic 参数，毛玻璃生效时中央内容区 panel_alpha=0 真透明（暗度仍由 gradient_color alpha 控制，与内容透明度解耦）；main.rs 传 self.plugin_bg_active。
  2. **背景效果入口移入单插件配置区（黄框）**：删除插件页顶部工具栏「开启:毛玻璃/半透明/恢复默认背景」按钮组与「毛玻璃暗度」滑杆（总设置）；每张插件卡片「配置」折叠区内新增背景效果控件——「毛玻璃」「半透明」「恢复默认背景」按钮 +「毛玻璃暗度」滑杆，读写该插件 configs 保留键 bg_style/bg_alpha（脚本 xmst_config_get/set 同读同写），改动经 configs_dirty 落盘；滑杆 changed 且毛玻璃已生效时即时重绘。
  3. **脚本触发链路带插件名**：PluginMsg::SetBg 由 SetBg(BgStyle) 改为 SetBg(String, BgStyle)（携带请求插件名）；PluginManager.bg_request 同改；tick_plugins 消费时优先取该插件 configs 的 bg_alpha，无则回退全局 cfg.plugin_bg_alpha。
  4. **demo 插件闭环**：data/plugins/xmst-frosted-glass-demo.zip 与 dist 同款 zip 的 main.rhai 更新——on_server_started 读 xmst_config_get("bg_style") 决定 acrylic/translucent/default（空值回退 acrylic 默认），UI 单插件配置与脚本触发行为一致。
  5. **文件**：src/theme.rs、src/main.rs、src/plugins.rs、data/plugins/xmst-frosted-glass-demo.zip、dist\data\plugins\xmst-frosted-glass-demo.zip、F:\XMST\HANDOVER.md。
  6. **验证**：cargo check 0 error；release 构建 Finished 3m23s；dist\XMST-0.1.0-alpha.exe SHA256 471E65556D8C1AB1EBE26E2DF55E7272D7C50626BD3404FBC70A38FA3B08A06E 与 release 产物一致，LastWriteTime 08:06:39；旧版已备份 dist\backup\20260929_080815\。
- 2026-09-29（单 exe 发布策略落地 + 双交接文档合并，用户指令：每次更新后都构建产出、不再双 exe、产出名「XMST-版本号.exe」、之后仅维护 HANDOVER.md）：
  1. **release 构建**：cargo build --release 后台完成 07:35:46 Finished（0 error，71 warnings 历史遗留），产物 `target\release\xmst.exe` 15,215,104 字节。
  2. **单 exe 产出**：dist 已产出 `dist\XMST-0.1.0-alpha.exe`（版本号 0.1.0-alpha 取 Cargo.toml version），SHA256 `C0574A458A1F552C50F6541E8E5104C2D84CA079B4E2E44C8969300B178308EF`，与 release 产物一致；LastWriteTime 07:35:46 晚于源码改动，磁盘实证通过。
  3. **双 exe 退役**：`dist\xmst.exe` 与 `dist\修暝的服务器工具.exe`（各 15,190,528 字节，07:01:49 旧版）已删除（进回收站），旧版已备份 `dist\backup\20260929_074009\`。
  4. **文档合并**：HANDOVER_综合交接与经验总档.md 全部内容已并入本文档第 9 章（关键架构决策 / 全部历史错误与经验教训 / 各阶段交付汇总 / 文件存储位置规范 / 后续开发防错清单 / 当前已知问题与待办，含过时点注释），原文件退役归档，此后仅维护本文档。
  5. **发布流程更新（此后每次改动必走）**：改后 `cargo check` → `cargo build --release` 后台化 → 产出复制为 `dist\XMST-<版本号>.exe`（单文件，不再双 exe）→ 旧版先备份 `dist\backup\`（保留最近 3 份）→ SHA256 + LastWriteTime 核对 → 本文档最近变更记录倒序插入。
  6. **文件**：F:\XMST\HANDOVER.md、F:\XMST\HANDOVER_综合交接与经验总档.md（归档标注）、dist\XMST-0.1.0-alpha.exe、dist\backup\20260929_074009\。
- 2026-09-29（用户 5 项反馈落地：圆角幅度拆分 / 背景图不再叠加侧栏按键 / 删除「保存配色」按钮 / 界面字号移位 / 插件单独配置 + 毛玻璃与透明度入口，cargo check 0 error（71 warnings 历史遗留），本次改动零新增 warning）：
  1. **圆角幅度拆分（①）**：设置-界面页原「圆角幅度」滑杆拆分为「控件圆角幅度」（corner_scale，仅控件/菜单）与「窗口圆角幅度」（window_corner_scale，整窗圆角）两个独立滑杆，互不联动；config.rs 新增 window_corner_scale 字段（默认 1.0，serde default = default_window_corner_scale）；theme.rs apply 已接收 window_corner_scale 用于整窗圆角半径计算；设置页各滑杆 changed 即 save_config。
  2. **背景图不叠按键（②）**：5 个 SidePanel（nav / server_list / tunnel_side / settings_side / dl_comm_nav）补 frame(egui::Frame::side_top_panel(&style).fill(visuals.window_fill)) 不透明填充，背景图不再透到侧栏导航/按键上；仅 CentralPanel 内容区继续透出壁纸。
  3. **删除「保存配色」按钮（③）**：自定义配色 RGB 为实时预览 + 即时落盘，DragValue changed() 即 save_config，删除「保存配色」按钮与「修改后点击保存配色生效」提示，改为「配色修改实时生效并自动保存」。
  4. **界面字号移位（④）**：界面字号块从主题模式与自定义配色之间移至自定义配色下方（背景图分区之前），不再插在自定义配色中间。
  5. **插件单独配置（⑤）**：PluginManager 新增 configs: Arc<Mutex<HashMap<String, HashMap<String, String>>>>（按插件名隔离的键值配置）+ configs_dirty 脏标记；PluginMsg 新增 ConfigDirty；rhai 引擎注册 xmst_config_get(key)/xmst_config_set(key, value)（CURRENT_PLUGIN 线程局部限定当前插件，脚本写入即发 ConfigDirty）；插件页每张卡片新增「配置」折叠区（查看/改值/删除已有项，新增 key，改动经 configs_dirty 在 tick_plugins 同步回 cfg.plugin_configs 并 save_config 落盘）。
  6. **毛玻璃/半透明入口 + 透明度（⑤）**：插件页新增背景效果控制区——「毛玻璃」/「半透明」/「恢复默认背景」按钮（bg_cmd 走既有 apply_bg 链路）+「毛玻璃暗度」滑杆（cfg.plugin_bg_alpha 默认 0.8，0.0-1.0）；plugins.rs apply_acrylic 签名扩展 alpha: u8，gradient_color 由固定 0xCC 改为 (alpha as u32)<<24；apply_bg 毛玻璃分支按 plugin_bg_alpha 应用暗度，滑杆 changed 时若毛玻璃已生效即时重绘；文案注明「透明度仅对毛玻璃生效」。
  7. **文件**：src/config.rs、src/main.rs、src/plugins.rs、F:\XMST\HANDOVER.md。
  8. **验证**：cargo check 0 error，71 warnings 均为历史遗留（unused/mut/noop clone 等），与本次改动相关关键词（window_corner_scale/configs_dirty/plugin_cfg/apply_acrylic/plugin_bg_alpha/xmst_config/ConfigDirty）零告警；未构建 release（如需可再执行 cargo build --release 覆盖 dist 双 exe）。
- 2026-09-29（自定义配色升格为独立第三主题模式 + 恢复「控件圆角」命名并新增独立「窗口圆角」 + 整窗圆角改系统区域修复遮罩未生效，cargo build 后台 55s Finished（71 warnings 0 error），target\debug\xmst.exe 已重建 06:37）：
  1. **自定义配色升格第三模式（用户澄清）**：设置-界面页主题单选恢复为 日间/夜间/自定义配色 三选一（radio_value light/dark/custom）；删除原 custom_colors 覆盖开关（旧配置兼容：custom_colors=true 且非 custom 时 theme_mode 升格 "custom"，字段保留防旧配置反序列化丢失）；标题栏主题按钮改三态轮转 light→dark→custom→light（custom 显示调色板图标）；文字颜色按背景 RGB 自动反算黑白深浅（theme.rs target_colors custom 分支既有 luma>140 浅背景深字/深背景浅字逻辑随模式生效）；新增 App::theme_is_light() 统一判亮（light 恒亮、custom 按背景 luma、其余按暗），App::fg 与 4 处局部 fg 闭包改走 theme_is_light。
  2. **控件圆角命名恢复 + 窗口圆角新增（用户澄清）**：设置页「启用窗口圆角」恢复为「启用控件圆角」（继续控制 widget/window/menu Rounding + corner_scale 幅度）；新增独立「启用窗口圆角」（config.rs 新字段 window_round_corners 默认 true）；theme.rs apply() 签名扩展 window_round_corners 参数并返回整窗圆角半径（控件/菜单圆角仍由 round_corners 控制）；幅度滑杆条件放宽为 控件圆角 || 窗口圆角 任一开启时显示。
  3. **整窗遮罩未生效修复（根因）**：无边框不透明窗口上 Foreground 层画 bg 色角遮罩与背景同色，无法裁出圆角（此前视觉无效果的根因），删除 theme.rs 四角 PathShape 遮罩；改由 Win32 系统区域实现——apply() 返回半径，main.rs apply_window_round_region() 用 CreateRoundRectRgn+SetWindowRgn 裁剪整窗（内容/面板/背景图一并圆角，r_px 按 DPI 换算，缓存 (r_px,w,h) 避免每帧重复 SetWindowRgn，r=0 恢复直角）。
  4. **编译踩坑**：winapi um::winuser 无 CreateRoundRectRgn（实际位于 um::wingdi，winapi features 需增 "wingdi"）；fg 闭包捕获 self.theme_is_light() 触发 E0502（self 被闭包整体借用，后续可变借用失败），改预取 bool 后闭包只捕获 bool。
  5. **文件**：src/config.rs、src/main.rs、src/theme.rs、Cargo.toml、F:\XMST\HANDOVER.md。
  6. **release 覆盖**：cargo build --release 后台 4m17s Finished（71 warnings 0 error）；dist 双 exe 已重建覆盖一致 07:01，15,190,528 字节，SHA256 1F0C56A3F47CB22773F6B4957B6347C9C30E23D20ECAB573CB5880933DD57F62（双 exe 与 release 产物一致）；覆盖前旧双 exe（06:09，15,179,264 字节）已备份 backup\20260929_065640\。
- 2026-09-29（删除「跟随系统/自动化配色」，主题收敛日/夜且默认夜间；圆角命名修正为「窗口圆角」；用户澄清语义为「自定义配色」（可调背景/强调色）而非自动化配色，cargo check 0 error、cargo build --release 3m10s Finished（71 warnings 0 error），dist 双 exe 已重建覆盖一致 06:09，15,179,264 字节，SHA256 与 release 产物一致）：
  1. **删除跟随系统/自动化配色（用户指令）**：config.rs 删除 theme_follow_system 字段与 default_theme_follow_system 函数；main.rs 删除 system_light_theme()（注册表 AppsUseLightTheme 读取）、sync_system_theme_now()、App 字段 last_sys_theme_check/sys_light；tick_theme 去掉 auto 分支与 3s 轮询，直接按 cfg.theme_mode 决定色板方向；标题栏主题按钮去掉点击关跟随与 auto 方向判断（is_light_theme = theme_mode=="light"）；设置-界面页主题单选由 日间/夜间/自动化配色（跟随系统） 三选一收敛为 日间/夜间 两选一（radio_value 保留 light/dark，删除 auto 项与其 save 逻辑）。
  2. **默认夜间**：default_theme_mode() 本就返回 "dark"，随字段删除自动生效；旧配置兼容——load_config 检测历史 theme_mode=="auto" 一律迁移为 "dark"（不迁移则旧用户启动后 auto 无分支可走，等效夜间）。
  3. **自定义配色语义对齐（用户澄清）**：「自定义配色（覆盖强调色/背景色）+ 强调色 RGB + 背景色 RGB + 保存配色」功能完整保留，即用户所说「可以调整背景和强调色的那个东西」；背景图选择/编辑/清除 + 透明度同批保留。
  4. **圆角命名修正（用户未识别功能）**：功能本身已实现且生效——theme.rs apply() 同时控制控件/widget/menu Rounding（scale 0=直角）与 Foreground 层整窗四角 bg 色 1/4 弧遮罩（r=12*scale，clamp 48），截图四角可见圆角；用户未看到是因设置页标签写作「启用控件圆角」，已改为「启用窗口圆角」+ 圆角幅度 Slider 保留，与整窗圆角语义对齐。
  5. **文件**：src/config.rs、src/main.rs、F:\XMST\HANDOVER.md；theme.rs 本次未改动（圆角实现核对无 bug）。备份见 F:\XMST\backup\（dist exe 覆盖前旧双 exe 时间戳 20260929_0609）。
- 2026-09-29（用户 8 项 UI 反馈全部落地：字号待应用 / 整窗圆角遮罩 / 背景图不叠按键 / 默认强调色 28,150,130 / 自动化配色第三模式 + 文字自动对比 / 边缘 resize 手柄 / 导航竖线语义确认 / 左对齐修复，各批 cargo check 0 error，cargo build --release 后台 3m13s Finished（72 warnings 0 error），dist 双 exe 已重建覆盖一致 05:31，15,180,800 字节，SHA256 777B554BA75F1AAC2EEC9DD1ECAA3483AD4389F7F7BE8D86FA3A7B46707CB3AD）：
  1. **字号改为「应用」按钮生效（①）**：设置-界面页字号 Slider 拖动只写 pending_font_scale（新字段），点「应用」才写 cfg.ui_font_scale 并 save_config（重置按钮同理），不再实时缩放；每帧 want_ppp 仍按 cfg 计算，应用后下一帧即生效。
  2. **整窗圆角遮罩（②）**：theme.rs apply() 末尾新增四角遮罩绘制——按 round_corners/corner_scale 在 Foreground 层画 4 个 bg 色 1/4 弧 PathShape（r=12*scale，clamp 48），把整窗切成圆角剪影（含背景图）；标题栏按钮距右缘 12px 不会被遮罩盖住。
  3. **背景图不叠按键（③）**：theme.rs apply() 将 window_fill 改为不透明 panel 色、extreme_bg_color 改为不透明 bg 色（原为半透明），SidePanel/TopBottomPanel/Window/menu 均不再透出背景图；仅 CentralPanel（panel_fill 半透明）透出壁纸，按键/导航/面板不再被背景图叠加。
  4. **默认强调色改 28,150,130（④）**：theme.rs dark.accent=(28,150,130)、light 压暗 (45,75,65)；删除残留 #22D3EE 硬编码——main.rs 导航 hover 背景/滑块/选中竖条与设置-插件页选中卡片的 5 处 34,211,238 全部改读 self.theme_target.accent（自定义配色实时跟随）；target_colors 注释同步更新。
  5. **自动化配色第三模式 + 文字自动对比（⑤）**：设置-界面页主题单选扩展为 日间/夜间/自动化配色（跟随系统）三选一（radio_value），auto 与日/夜为切换关系而非共存；theme.rs target_colors 末尾按背景 RGB 亮度自动切换文字深浅（luma>140 深字 / ≤140 浅字，widget 底色同步调整），自定义背景与 auto 模式下文字始终可读。
  6. **导航竖线语义（⑥）**：左侧导航选中态 3px 圆角竖条 + nav_anim 插值滑块（hover 半透明强调色背景）为「当前页选中指示器」，非滚动条、不可拖动；配合批次2 已移除的 35 处 scroll_bar_visibility(AlwaysVisible) 与 solid 传统滚动条，确认图1 中不可拖动的强调色竖线即选中指示。
  7. **窗口边缘 resize 手柄（⑦）**：App 新增 handle_edge_resize——无边框窗口（decorations=false）系统边缘 resize 失效，现按 6px 命中带检测鼠标（非最大化时），四边/四角按下即发 ViewportCommand::BeginResize，光标同步切 Resize 系列；顶部边留给标题栏拖拽移动窗口，不做 resize。
  8. **字小左对齐（⑧）**：批次3 已把 6 处居中容器 pos2(_avail.center().x - _w*0.5) 改为 pos2(_avail.left(), _avail.top())，字体小时内容贴合左侧不再大片留白。
  - 各批 cargo check 0 error；备份见 F:\XMST\backup\（20260929 批次）。
  - 文件：src/theme.rs、src/main.rs、F:\XMST\HANDOVER.md。
- 2026-09-29（UI 重构六批收尾：主题系统重构 / 字号 / 1100 居中 / 滚动条 / 导航 / 顶部栏 / 设置三页 / 插件卡片 / 下载三栏 / Spark 页，cargo build --release 后台 3m14s Finished（71 warnings 0 error），dist 双 exe 已重建覆盖 02:20，15,176,704 字节，SHA256 52C62830936ED4F8055D3C270A6C3DA65B301413A68FF0C9D78841B6C7891060）：
  1. **主题系统重构**：删除预设配色下拉，只保留日/夜模式；右上角新增太阳/月亮切换按钮（点击即关闭跟随系统）；设置-界面页新增「跟随系统」默认开启（theme_follow_system 新字段）；强调色统一 #22D3EE。
  2. **字号比例化**：ui_font_scale 默认 14（范围 11-20），正文 14px 起、日志等宽 13px 起。
  3. **全页面 max-width 1100px 居中**：设置/插件/下载/日志等页统一内容容器，解决左右留白不一致。
  4. **滚动条增强**：hover 亮度提升，槽位与轨道可区分。
  5. **左侧导航二级缩进列表**：子导航缩进 + 选中 3px 左侧竖条 + 轻微背景色，去徽章样式。
  6. **顶部栏三区**：标题/版本号与状态提示分左/中/右三区，状态提示居中并加图标；窗口按钮 hover 背景色块（Win 风格）。
  7. **设置三页改造**（通用/界面/日志）：区块间距统一 24px + 统一底部分割线；说明文字缩进对齐选项首行；高风险 Defender 红色边框卡片包裹；控件组「标签在上、控件在下」；滑杆+数值框合成组件；日志预览固定高 300px 等高卡片 + 14px 等宽字体行高 1.6；统计改三个小型统计块（已存/上限/清空）。
  8. **插件页卡片化**：名称 + 状态徽章（红=停止/绿=运行）+ 描述一行 + 按钮右对齐。
  9. **下载页三栏**（第五批）：自定义下载表单改两列 Grid（dl_custom_form_grid，main.rs L12923-12997）；过滤栏控件统一 26px 高、8px 间距（L13236-13369）；列表行改左侧信息区 + 竖分割线 + 右侧固定 200px 操作列（含打开目录按钮）。
  10. **Spark 页改进**（第六批，spark_analysis.rs）：新增 format_tick（K/M 缩写，L499）；draw_series 空数据占位图（空心圆+暂无数据，L1491-1510）；三处 ProgressBar 移除内嵌百分比改外部标签且 0% 条目不绘制（L1032/1135/1207）；阈值与标题用 format_tick。
  - 六批各自 cargo check 0 error；每批备份 F:\XMST\backup\（时间戳前缀 20260929_014200_main.rs / 20260929_020000_spark_analysis.rs）。
  - 覆盖前 dist 双 exe SHA256 E11FDA2A75EBD1995B233289828B96A819FF74FA82C7467D1FDEE6DC34E958D0（15,149,056 B）已备份 dist\backup\xmst_20260929_022049.bak / 修暝的服务器工具_20260929_022049.bak；覆盖后两份 SHA256 与 release 产物一致 52C62830…。- 2026-09-28（亮色模式对比度修复 + 另一AI修改建议评估：弱文字(.weak)在亮色模式不可见——根因 egui 0.29 weak_text_color=gray_out(正文)，混色目标 widgets.noninteractive.weak_bg_fill 亮色默认 248(近白)，已改亮色下置 from_gray(120)（theme.rs apply）；预设配色亮色模式原样套用深色背景导致"深底深字"，已改为亮色仅取压暗强调色（target_colors）；hyperlink/选区/光标亮色统一 light_adapt(accent) 防自定义浅强调色不可读；导航栏普通项文字 205→170 亮色更清晰。cargo build --release 成功，dist 双 exe 已重建覆盖 14:30 SHA256 0B24CD50BE155DD66C87CCC8061015C2CD4B5A8141757E79411E47AA78EF2155）：
  - theme.rs：apply() 增亮色 noninteractive.weak_bg_fill=120、accent_fg 压暗派生（hyperlink/selection/text_cursor）；target_colors() 增亮色预设分支（仅 accent light_adapt）。
  - main.rs：导航栏非激活项 from_rgb(205,205,205)→from_rgb(170,170,178)（L5500）。
  - 评估结论：另一AI修改建议 10 条中，对比度类（弱文字/强调色/预设深底）已采纳修复；字号14/最大宽1100/强调色统一/导航改缩进列表/顶部三区/滚动条增强/结构重构等为风格性重构，暂不采纳（影响面大、非本次可读性问题）。
- 2026-09-28（Spark 分析页图1/图2 反馈第二轮改造：折叠概览 + 模组定位打分制 + 萌新可读 + 去交替背景，cargo build --release 后台 3m16s Finished（65 warnings 0 error），dist 双 exe 已重建覆盖一致 14:29 SHA256 0B24CD50BE155DD66C87CCC8061015C2CD4B5A8141757E79411E47AA78EF2155）：
  1. **概览改可折叠展开**：ui_overview 重写为「结论横幅（严重/偏高/正常着色）+ 摘要卡网格 + 可展开指标区」；TPS/MSPT/堆内存/CPU/GC/实体 六项分别用 ui_metric_collapse 折叠（顶部 level 徽标「⚠ 严重/▲ 偏高/✓ 正常」+ 主值，展开后内嵌波动曲线 + 萌新解读提示）；实体按维度分列（entity_series 曲线 + 维度明细，dim_friendly_name 适配原版三维度/裸名/未知维度原样显示 ID）；概览内可展开的曲线复用 draw_series（阈值线 + 红点卡顿段高亮），与「波动曲线」页签同款。
  2. **模组定位精度（打分制）**：match_mod_jar 重写为 score_match 打分——3 分全等 / 2 分候选以来源为前缀 / 1 分来源以候选为前缀（带分隔符）/ 0 分双向包含，取最高分；Carpet 扩展包（如 carpet-extra、carpet-tis-addition）优先命中自身，不再全归 Carpet 本体；仍优先匹配元数据别名（fabric.mod.json id/name、mods.toml modId/displayName），来源名 <3 字符仅完全相等。
  3. **萌新可读**：TPS/MSPT/堆内存/CPU/GC 卡按阈值分 0/1/2 级（灰色/黄色/红色）并附中文判定（如「严重：MSPT 超 50ms，玩家会明显卡顿」）；模组性能 TOP 与卡顿热点 TOP 去 striped 交替背景（ProgressBar 平滑填充），改按占比列显示进度条 + 萌新化列说明（列头加说明提示）；卡顿热点列序调整为 方法/占比/自耗时/来源/类型/操作。
  4. **概览排版与波形整合**：SparkSummary 新增 heap_series/cpu_series/cpu_sys_series/entity_series/entity_total/entity_worlds；collect_windows/collect_metrics 补充堆内存(MB)/CPU(进程/系统)/实体总数/各维度实体 时间序列收集；「波动曲线」页签保留独立入口但曲线数据与概览展开区共用。
  5. **编译踩坑**：ProgressBar::new 参数为 f32（egui 0.29），pct 是 f64 需 `(pct / 100.0) as f32` 三处（1011/1106/1170 行）；实体总数 i64 转换（419/422 行）；emoji（📊/💡）在 egui 默认字体可能缺字形显示豆腐块，改纯文字「【结论】/提示：」。
  6. **文件**：src/spark_analysis.rs、F:\XMST\HANDOVER.md。
- 2026-09-28（三项 UI 反馈：Spark 概览超界换行 / 窗口自定义缩放 / 左侧栏平滑折叠，cargo check Finished、cargo build --release Finished（0 error，仅 Cargo.toml unused manifest key 警告），dist 双 exe 已重建覆盖一致（12:40，15,111,680 字节））：
  1. **Spark 概览超界修复**：spark_analysis.rs 概览卡片弃用 Grid 自适应列（Grid 按内容自然宽撑列，长文本「世界 the_nether/overworld/…」「CPU 进程/系统」等把列撑宽超出窗口右边界），改手动分行——按 available_width 实时算列数（<560px 2 列）与列宽 col_w=(可用宽-间距)/列数，封顶 340px，每卡片 allocate_ui 固定列宽渲染，长文本在卡片内换行不再超界；ui_card 的 set_min_width 由固定 150.0 改为 available_width().min(150.0)，防窄窗二次撑宽。
  2. **窗口自定义缩放**：main.rs NativeOptions viewport 新增 .with_resizable(true)（此前仅 with_inner_size 1180x760 / with_min_inner_size 960x600 / with_decorations(false)，无边框窗口无法拖拽边缘调整，只能正常/最大化）。
  3. **左侧栏平滑折叠**：App 新增 servers_collapsed/servers_anim 字段；ui_servers 侧栏折叠动画（沿用 nav_anim 指数逼近风格，系数 0.18，ui_animations 关闭时直接切换）：展开 240px ↔ 折叠 44px 仅图标；折叠态每服务器显示收藏星标 + 名称首字符图标 + 运行状态圆点（绿=运行/灰=停止），hover tooltip 显示名称，右键菜单（重命名/立即备份/移除）与点击切换服务器保留；左下角新增「◀ 收起侧栏 / ▶」按钮，展开态底部「创建新服务器/添加服务器目录」折叠后图标化为 ＋/📂；展开动画完成后恢复 resizable 可拖宽（width_range 44..=480）。
  4. **文件**：src/spark_analysis.rs、src/main.rs、F:\XMST\HANDOVER.md。
- 2026-09-28（Spark 分析结果页排版重构 + 玩家定位功能 + 混淆映射说明，cargo build --release 后台 3m21s Finished（65 warnings 0 error），dist 双 exe 已重建覆盖 SHA256 一致 3B23C6B042B798CC9E52B3BAC0798E0D84D007DAF143513E7D5B64EA6F3878B0）：
  1. **分析结果页签化**：spark_analysis.rs 新增 SparkViewTab（概览/波动曲线/模组性能/卡顿热点）与 ui_analysis 页签栏，仿服务器顶栏可切换式布局替代单页纵向堆叠——非最大化窗口不再超界；内容区套独立 ScrollArea；概览卡片按可用宽度自适应 2/3 列（<560px 用 2 列）；元信息仅概览页展示。
  2. **卡顿源→模组定位**：SparkSummary 新增 mod_hots（模组→热点明细，每模组最多 20 条，aggregate_tree 归因时同步填充）；SparkViewTab::Mods 页模组 TOP 行新增「📍 定位」（按 match_mod_jar 在 mods jar 索引匹配，找不到则禁用）与「详情」（展开该模组热点方法明细 + 类型分布 + 一句定位建议）；Hotspots 页热点行新增「📍 定位」。match_mod_jar 匹配 jar 名去扩展名与元数据别名（fabric.mod.json id/name、mods.toml/neoforge.mods.toml modId/displayName），来源名 <3 字符只允许完全相等防误匹配。
  3. **跳转文件浏览**：RuntimeState 新增 mod_jar_index（jar 定位索引缓存，惰性 scan_mod_jar_index 扫描 mods 目录解析 zip 元数据，复用 zip/serde_json/toml 依赖）与 file_scroll_to；locate_mod_jar 切到 Files 页 mods 页签 + 搜索词置 jar 名 + refresh_file_list + toast；ui_files 目录/文件行渲染后按 file_scroll_to 匹配 scroll_to_me(Align::Center) 滚动定位并清空目标。
  4. **编译踩坑**：① hot_top 用 nodes.into_iter().take(20) 会 move nodes，后段再 sort_by 报 E0382——改 nodes.iter().take(20) 克隆数据，第二段再 into_iter 消费；② 详情区 if let Some(detail)=mod_detail 解引用后在 ui.horizontal 闭包内 *mod_detail=None 报 E0500——改用 close_detail 局部标志、闭包外写回。
  5. **映射说明**（对应 class_10209/method_64148 问题）：1.21.11 环境 class_10209 = Yarn 反混淆 net/minecraft/util/profiler/Profilers，method_64148 = deactivate()V（关闭性能分析器）；该类还含 activate()/toggle() 等，供服务端 spark/内置 profiler 调用，属正常游戏逻辑。
  6. **文件**：src/spark_analysis.rs、src/main.rs、F:\XMST\HANDOVER.md。
- 2026-09-28（修复：Spark 输出文件点击分析报「os error 3」——scan_spark_files 存的是相对子目录 config\spark / plugins\spark，UI 点击时拼出的 full 未加服务器绝对路径，fs::read 按进程工作目录解析失败；现点击时以 sc.dir.join(full) 补全绝对路径再解析，选中态比较同步改为绝对路径，cargo check 32.89s Finished）：文件 main.rs ui_special 文件列表点击处。
- 2026-09-28（问题2：Spark 性能分析 UI 排版重构——ui_special 改为总折叠「Spark 性能分析」+ 子折叠（安装检测/性能分析/输出文件/分析结果预览），左右分栏：左侧 300px 分析控制（采样时长 DragValue 5..=3600 + 开始分析 + 运行中进度条/强制停止/完成提示），右侧输出文件列表与解析预览；新增 tick_spark_prof（到期自动发送 spark profiler stop --save-to-file，2.5s 后自动刷新输出文件）、spark_send_cmd（stdin 发送）；新增 SparkProfiling 结构体与 Runtime.spark_prof/spark_prof_refresh_at/spark_prof_secs 字段；cargo check 通过）：文件 main.rs ui_special/ui_special_body/ui_spark_control/ui_spark_preview/spark_send_cmd/tick_spark_prof、Runtime 结构。
- 2026-09-28（问题3：特殊功能（Spark 分析）测试功能转正式——features.rs BETA_SPECIAL/FEATURE_SPARK 注册表项 group Beta→Server、default_enabled false→true，设置页测试功能列表移除 BETA_SPECIAL 项（beta_list 4→3），特殊功能页未启用时点击「启用 Spark 分析」直接 set_feature 不再弹 beta_confirm 确认窗；cargo check 通过）：文件 features.rs、main.rs 设置页测试功能列表、ui_special 启用按钮。
- 2026-09-27（阶段18：Spark 文件解析与分析 UI 完成：prost 编译 proto + gzip 解压 + protobuf 解码，分析 UI 全量接入（概览卡阈值着色 / TPS-MSPT 波动曲线 / 模组性能 TOP / 卡顿热点 TOP / 无数据显示“无数据”），cargo check 通过、cargo build --release 后台 4m21s Finished，dist 双 exe 覆盖一致 SHA256 575CD5AD590EE21AE7F947323A8A36A03DABABE32E09A730F6FF943B7300B4E6）
  1. **proto 编译**：Cargo.toml build-dependencies 增加 protoc-bin-vendored="3"（系统无 protoc），build.rs 设置 PROTOC 环境变量后 compile_protos（OUT_DIR 生成扁平化 spark.rs，无 spark:: 模块包裹）；spark_analysis.rs include OUT_DIR 并去除 spark:: 前缀引用。
  2. **解析管线**：.sparkhealth → HealthData（TPS 多窗口 / MSPT mean-min-median-p95-max / CPU / 内存 / GC / 实体 / 区块 / 玩家 + WindowStatistics 窗口序列），.sparkprofile → SamplerData（time_window_statistics + ThreadNode 扁平调用树 + class_sources 模组来源映射）；gzip 魔数（1f 8b）判断解压，非 gzip 或解压失败回退原始字节；解码失败给明确提示（损坏 / spark 版本不兼容）。
  3. **字段核对（踩坑）**：Metrics 时间序列字段为 timestamp_deltas_ms（非 timestamp_deltas）；StackTraceNode 无嵌套 children（proto reserved），调用树为 ThreadNode.children 扁平数组 + children_refs 下标引用，self time 直接取节点 times 之和（共享节点只出现一次，天然免重复计数，删除原 dfs_node 递归）；egui 0.29 Frame 圆角用 rounding()（corner_radius 已废弃）；HashMap keys 借用避免 move 后再次借用。
  4. **分析 UI**（spark_analysis.rs ui_analysis 渲染，文件列表点击条目触发 parse_spark_file 缓存 SparkAnalysisState）：概览卡片阈值着色（TPS<18 / MSPT>50 警示红底红字）；TPS / MSPT 波动曲线（网格 + 阈值线 + 折线 + 卡顿段红点与红带高亮，标注起始窗口与峰值）；模组性能 TOP（按 class_sources 聚合自耗时占比，最多 15 条）；卡顿热点 TOP（自耗时最高的方法 / 类，附实体 / 区块 / 红石 / 方块 / AI寻路 / 网络 / 流体类型标签，最多 20 条）；各区块无数据时显示“无数据”而非报错。
  5. **文件**：新增 src/spark_analysis.rs（797 行）；修改 src/main.rs（ui_special 集成分析区 / parse_spark_file / 未启用文案更新）、Cargo.toml（protoc-bin-vendored）、build.rs（vendored protoc）。
- 2026-09-27（阶段17：新增“特殊功能-Spark 分析”架构阶段——详情页“特殊功能”页签 + Spark 可用性检测 + 输出文件列表 + proto 解析依赖，cargo check 8.84s Finished、cargo build --release 后台执行 3m28s Finished（1 warnings 0 error），dist 双 exe 已重建（22:34，4,889,472 字节，SHA256 一致 017F513EEFFD0A32FBFC5A1DEAFDA19910D7DB98D7DA7745646DDA4B37943415）：
  1. **「特殊功能」页签**：ServerTab 新增 Special 变量；测试功能体系新增 BETA_SPECIAL（总开关，控制页签显示，默认关）与 FEATURE_SPARK（Spark 子开关，默认关）两项，features.rs REGISTRY 登记 order 90/91；延用「名称·状态·启用/禁用」模式与启用确认弹窗（详情页与设置页共用 beta_confirm 流程）；总开关关闭则页签不显示，子开关关闭则页内提示未启用并提供启用按钮。  2. **Spark 可用性检测**：detect_spark 扫描 服务器目录\mods\*.jar 文件名（不区分大小写）含 "spark" 判断已安装；不可用时隐藏 Spark 操作区并提示。  3. **输出文件列表**：scan_spark_files 读取 config\spark/ 与 plugins\spark/ 下 .sparkprofile/.sparkhealth 文件列表（文件名+大小+修改时间），ui_special 展示与刷新按钮；切换服务器/页签时清除缓存重新扫描。  4. **proto 解析依赖**：Cargo.toml 新增 flate2="1"、prost="0.13"；src/spark_proto/ 放入官方 spark.proto（6338B）与 spark_sampler.proto（2410B，源 github.com/lucko/spark spark-common/src/main/proto）；有 schema，解析代码下一批实现。  5. **文件**：src/main.rs、src/features.rs、Cargo.toml、src/spark_proto/spark.proto、src/spark_proto/spark_sampler.proto、F:\XMST\HANDOVER.md。
- 2026-09-27（阶段16：四个界面问题修复①-④，cargo check 6.45s Finished、cargo build --release 后台执行 3m39s Finished（1 warnings 0 error），dist 两份 exe 已重建覆盖（21:54，4,862,848 字节，SHA256 一致 AEBDF1508DC888A995F56E099E79FE26E502E4CE30AB484D37D3ED7588D21E46）：
  1. **①左侧分类点击收起详情页**：ui_dl_modrinth 左侧分类导航点击由"保留详情"改为清空 mod_detail_id/mod_detail_hit/mod_versions/mod_ver_*/mod_translate_* 等全部详情状态，列表切换到该分类重新搜索加载。  2. **②快照版本默认隐藏 + 显示开关**：DlUiState 新增 mod_show_snapshot: bool（默认 false，默认隐藏 snapshot- 版本）；加载器筛选行（fabric/neoforge 那排）右侧新增「显示快照版本」selectable_label 开关；ui_mod_detail_body 版本列表用 mod_show_snapshot 过滤 version_number/filename 以 "snapshot-" 开头的条目，全量非空但被过滤清空时提示用户开启开关。  3. **③下载进度条缩短 30%**：ui_mod_detail_body 下载区进度条用 available_width*0.70 计算 desired_width（最大 160px），总宽缩短 30%。  4. **④文件浏览页删除按钮 + 二次确认 + 回收站**：ServerRuntime 新增 file_delete_confirm: Option<String>（Default None）；目录行/非目录行「打开」按钮后新增「删除」按钮；ui_files 末尾新增居中二次确认 Window（显示文件名 + 取消/确认删除，提示移入回收站可还原）；新增 delete_to_recycle_bin 方法（PowerShell Microsoft.VisualBasic.FileIO DeleteFile/DeleteDirectory + RecycleOption.SendToRecycleBin 删到回收站（禁止永久删除））；删除成功后清理回收站预览/子目录栈（删除的正是当前目录时退回上级）与 refresh_file_list。  5. **文件**：src/main.rs、F:\XMST\HANDOVER.md。- 2026-09-27（阶段15：模组社区下载四项反馈，cargo check 42.47s Finished、cargo build --release 后台执行 3m07s Finished（1 warnings 0 error），dist 两份 exe 已重建覆盖（21:12，4,851,584 字节，SHA256 一致 30711DBA0E298774E30E1D8E6955772A9743E36E2F34EAFFC03680295CEB69EF）：
  1. **①下载进度条移至下载区下方**：删除 ui_dl_community_search 函数末尾（页面最底部）的 mod_dl_progress 渲染块，改在 ui_mod_detail_body 右侧预览「下载/打开页面」按钮行之后渲染进度条 + 下载文件名；自定义下载页进度条原已在保存目录下方，未动。  2. **②详情页不退 + 数据包归类目录修复**：左侧分类导航点击仅更新 mod_nav/mod_project_type、清搜索结果重搜，显式保留 mod_detail_id/mod_detail_hit/mod_versions（详情页继续显示，加注释防回归）；mod_install_file 与 mod_install_project 的 datapack 归类由 "datapacks" 改为 "world/datapacks"（Path::join 拼接 <服务器>/world/datapacks），plugin/mods 不变；ui_dl_mod_target 两处提示文案同步改为 world/datapacks。  3. **③搜索版本置顶高亮**：DlUiState 新增 mod_detail_mc 字段；dl_mod_ver_load_start 打开详情时记录 mod_mc_version；ui_mod_detail_body 用稳定 sort_by_key 将匹配 MC 版本的组置顶，组头与组内版本行均橙色（255,170,60）加粗高亮。  4. **④加载器筛选自动跟随**：dl_mod_ver_load_start 打开详情时将 mod_ver_loader 置为搜索时的 mod_loader 值（如 neoforge）；模组无该加载器版本时由既有逻辑（加载后不在实际集合则重置"全部"）兜底。  5. **文件**：src/main.rs、F:\XMST\HANDOVER.md。- 2026-09-27（阶段14：服务器文件浏览页新增「排查客户端模组」，cargo build --release 后台执行 3m13s Finished（1 warnings 0 error），dist 两份 exe 已重建覆盖（20:27，4,840,320 字节，SHA256 一致 F4B7381D664EA27CA16791E4795F70191C502E743D72867F1E0F655AA1C0C481）：
  1. **排查客户端模组功能**（ServerTab::Files 文件浏览页）：mods 页签新增「排查客户端模组」按钮；点击先弹 egui::Window 二次确认（文案：客户端模组排查为启发式分析，结果不一定正确，请结合经验自行判断），确认后调用 analyze_client_mods(idx) 扫描当前文件浏览目录下所有 .jar。  2. **静态解析逻辑**（jar_is_client_mod，参考 MSL IsClientSideMod 原理，纯静态不加载模组）：fabric.mod.json 的 environment=="client" 判为客户端模组；META-INF/mods.toml / neoforge.mods.toml 按 [[mods]] 块解析 side，modId=minecraft 块优先否则取首个块，side=="CLIENT" 判为客户端模组；解析失败静默忽略视为非客户端模组。复用既有 zip="2" / serde_json / toml 依赖，未新增重依赖（zip::ZipArchive 既有使用先例）。  3. **标橙置顶 + 状态不持久化**：ServerRuntime 新增临时字段 file_client_mods: Option<Vec<String>>（None=未排查）与 file_client_mods_confirm；排查结果文件名单行标橙（255,170,60）并 hover 提示；排序置顶优先级：收藏 → 客户端模组 → 文件夹 → 名称方向（收藏组内/非收藏组内均将客户端模组排前）；离开文件浏览页（切 tab/切服务器/切页签）即清除标记恢复普通排序与颜色（clear_client_mod_marks / 页签点击清除），不进持久化配置。  4. **踩坑记录**：切服务器列表星标按钮的 ui.horizontal 闭包内直接调 self.clear_client_mod_marks() 触发 E0502（外层 sc=&self.cfg.servers[i] 不可变借用），改用 switch_server: Option<usize> 收集意图、循环外执行；cargo check 确认无 error、新代码区域无 warning 后才后台化 build。  5. **文件**：src/main.rs、F:\XMST\HANDOVER.md。- 2026-09-27（阶段13：四项反馈修复①-④ + HANDOVER.md 重建④，cargo build --release 后台执行 3m02s Finished（1 warnings 0 error），dist 两份 exe 已重建覆盖（19:50，4,676,992 字节，SHA256 一致 9FD849557A1B3A1734EF272484556AD2E69AAD9DD08D4244FA018C385437E6D2）：
  1. **①翻译缓存复用**：ui_mod_detail_header 译文渲染增加 !mod_translate_hidden 判断（隐藏不清缓存）；「翻译简介/隐藏翻译」按钮逻辑改为：有缓存时仅切换 mod_translate_hidden 标志、不再调用 modrinth::translate_text；翻译完成置 hidden=false；收起详情清理 hidden 状态。  2. **②搜索卡片点击穿透**：ui_mod_row 与 ui_mod_card 记录 title_rect/retry_rect/fav_rect；整行/整卡点击用 interact_pointer_pos 落点判断——落在收藏则收藏、落在标题/译文/「译」按钮则 None（不进详情）、其余 Detail，修复点名称/译文穿透进详情页。  3. **③分页数量 bug**：DlUiState 新增 mod_search_pending 标志（初始化 false）；dl_mod_search_start 的 busy 保护改为 pending 排队（busy 时置 true 返回，启动时重置 false）；ui_dl_community_search 搜索参数变更后若 mod_search_pending 且非 busy 则清 pending 并立即 dl_mod_search_start()，修复调整显示数量后只显示一个模组的问题。  4. **④HANDOVER.md 重建**：补齐截断原因分析（无卷影副本/无 git/写入链路截断证据）、章节 1（项目概述）/2（交付基线）/3（功能清单）/4（版本条件热点表）/5（已知问题与待复测）、阶段 0-10 变更记录（阶段 9/10 无记录如实标注）、试错速查 10-16；保留原未损坏内容不删改。  5. **文件**：src/main.rs、F:\XMST\HANDOVER.md。- 2026-09-27（阶段12：图1图2三项UI反馈落地，cargo build --release 后台执行 3m16s Finished（1 warnings 0 error）；dist\xmst.exe 已重建覆盖（18:33，4,675,968 字节），「修暝的服务器工具.exe」因运行中被占用未覆盖（PID 11184））：  1. **译文位置与样式**：ui_mod_detail_header 译文块移至操作行（打开仓库/MC百科/译）之前即按钮上方，RichText 去「译文:」前缀直接绿色字（旧操作行后灰字译文块已删）。  2. **列表加模组图标**：ui_mod_detail_header 水平布局左侧 mod_icon_ui(ui,&hit,40.0) 渲染图标（走既有 mod_icon_tex/pending/failed 状态），右侧垂直标题/作者/下载量/简介，与外部列表一致。  3. **搜索结果分页 + 数量控件**：modrinth.rs search_mods 加 limit(i32,clamp 1..=20)/offset(i64) 参数并返回 SearchResult{hits,total_hits}（total_hits 取 search 响应 total_hits 字段）；main.rs DlUiState 加 mod_page_size(默认20)/mod_page/mod_total_hits，过滤栏加载器后加「显示模组数量」DragValue(1..=20) 变更即重搜回第 1 页，结果统计用 total_hits，底部加分页条「上一页 / 第 x/total 页 / 下一页」切页调用 dl_mod_search_start 并置 mod_scroll_top。  4. **构建踩坑**：首次 build 报括号不匹配（过滤区插入漏 1 个 frame group 闭合）补上；随后 release 构建报 SearchResult 未定义——首次含乱码注释的 edit 失败未生效导致函数签名已改而结构体缺失，补上 SearchResult 定义；mod_search_shared 字段类型同步改 Result<SearchResult,String>，修完后 check/build 通过。  5. **文件**：src/main.rs、src/modrinth.rs、F:\XMST\HANDOVER.md。
- 2026-09-27（阶段11：详情页三项反馈 + 覆盖发布，cargo build --release 后台执行 2m37s Finished（0 warnings 0 error），dist 两份 exe 已重建覆盖（11:55，2,006,912 字节，SHA256 一致 933DC229E65F15A1B1A6232D9D7ED7D331E1D144C94717FD5B8F6B0C55511ECB）：
  1. **详情页三项反馈**（用户图 1/2/3 点名）：①版本列表竖排（弃 Grid 分列）；②加载器动态过滤——下拉选项取 mod_versions 各版本的 loaders 并集，按 fabric/forge/neoforge/quilt 固定顺序，未命中顺序的追加尾部；③头部按钮左移、版本行去安装按钮、预览页收「下载 + 打开页面」两按钮（dl_mod_open_page 用 cmd start 打开 Modrinth 项目页）；修改 ui_mod_detail_header 与 ui_mod_detail_body。  2. **构建踩坑**：阶段11 修改后首次 build 前台执行被 shell 超时截断，仅见 warning 行误判为成功，实际 error[E0277]: can't compare str with &str（src/main.rs:12460 .filter(|l| seen.iter().any(|s| s.as_str() == *l))，border.iter() 的 filter 闭包参数为 &&str，l 仍是 &str 导致 s.as_str() 比较失败）；修复为 s.as_str() == **l 后后台化 build（Start-Process + 轮询日志）成功；**教训：cargo build --release 必须后台化并核对日志出现 Finished 行 + exe LastWriteTime 晚于 main.rs，仅凭无 error 输出不能判定成功**。  3. **文件**：修改 src/main.rs、F:\XMST\HANDOVER.md。
- 2026-09-27（阶段8：模组社区界面遗留问题修复收尾：创建窗口关闭 + 搜索栏五项 + 卡片竖排 + 详情页六项，cargo build --release 通过（target x86_64-pc-windows-gnu，0 warnings 0 error），dist 两份 exe 已重建覆盖（12:46，1,968,512 字节，SHA256 一致 8CF3B78D3078EFA8A48836277B0EAD17AC00D7B655576776A9834FF7914F7343）：
  1. **创建服务器窗口关不掉**（用户点名）：ui_create_server 开窗新增 open 参数（初始 self.create_server.is_some()），!open 时置 create_server=None 兜底关闭；修复前点×仅标记未消费，窗口常驻。  2. **搜索栏五项改造**（用户点名）：hint 改 "Search..."；回车触发搜索（lost_focus && key_pressed(Enter)）；标签文本框改为下拉 ComboBox「选择标签」（15 个常用标签 + 清除标签，选中即搜索）；卡片网格与控件间距收紧（item_spacing y=3/x=6）；加载器筛选保留；
  3. **卡片竖排字母图标**（用户点名）：ui_mod_card 图标绘制弃用 painter.text（选框内单字符被 egui 竖排），改用 ui.fonts.layout_no_wrap + painter.galley 布局（loop 降字号至 9 或 galley 宽度达标），色块 34x34 圆角 8、首字母居中。  4. **详情页六项**（用户点名，本次完成）：①选择/过滤版本（既有分组 + 加载器筛选保留）；②显示完整简介（详情头下 wrap 全文展示）；③简单翻译（新增 modrinth::translate_text 走 translate.googleapis.com gtx 接口，DlUiState 新增 mod_translate_result/busy/error/shared，tick 消费线程回传，UI「翻译简介/隐藏翻译/翻译中/失败」）；④跳转链接（详情头「打开页面」按钮，cmd start 打开 Modrinth 项目页，slug 为空用 id）；⑤查看更新日志（版本行「日志」按钮，DlUiState 新增 mod_ver_log_open，changelog 是 HTML 经新增 strip_html 剥离标签/实体后 wrap 展示）；收起详情时清理翻译与日志展开状态；
  5. **构建踩坑**：cargo build --release 前台超 600s 被 shell 超时杀，须 Start-Process 后台化 + 轮询日志；category_options() 返回 Vec<String>，下拉里 *t（str）不能 Into<WidgetText>，需 t.clone()（tags == *t 比较也同步改 tags == t）；std::process::Command .args 数组要求同类型，URL 需 let url_ref: &str = &url;
  6. **文件**：修改 src/main.rs（ui_create_server / ui_dl_community_search / ui_mod_card / ui_mod_detail / strip_html 新增 / 翻译与日志状态字段 / tick 消费分支）、src/modrinth.rs（project_versions include_changelog / category_options / translate_text）、F:\XMST\HANDOVER.md。
- 2026-09-26（阶段C 四功能一次性完成：玩家级属性编辑 + SQLite 日志落库与轮转 + kill-on-drop 防孤儿 + frp 热重载，cargo check/build --release 通过（build 56 warnings 0 error），dist 两份 exe 已重建覆盖（21:51，1,912,192 字节，SHA256 一致 C2BD6C45…）：  1. **玩家级属性编辑**（对应 mctopai 未实现项）：App 新增 player_prop_name / player_prop_gamemode / player_prop_op 字段与 Default；玩家页新增「属性编辑」区（玩家名输入 + 游戏模式单选 survival/creative/adventure/spectator + OP 勾选），提交后通过控制台命令 gamemode <mode> <name> / op|deop <name> 下发。  2. **SQLite 日志落库 + 轮转**（对应 pmr 未实现项）：Cargo.toml 新增 rusqlite(bundled)；新增 src/logdb.rs（LogDb::open / insert_batch 带行数轮转 / recent 按来源过滤 / count / clear，DEFAULT_MAX_ROWS=50000）；main.rs 新增 mod logdb、App 字段 logdb/logdb_err，new() 时 open_logdb() 初始化 data/logs/xmst_logs.db（失败不阻塞，设置页显示错误）；update() 循环新增行增量写库（服务器日志 insert_batch(&srv, …) + 隧道日志 insert_batch(&format!("隧道:{tname}"), …)）；设置页新增「SQLite 日志库（阶段C）」区块（行数上限显示、当前行数、最近 5 条、清空按钮、错误展示）。  3. **kill-on-drop 防孤儿**（对应 ProcessKit-rs 未实现项）：process.rs 新增 create_kill_on_close_job()（Job Object + JOBOBJECT_EXTENDED_LIMIT_INFORMATION + JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE）；ManagedProcess 增加 _job 字段，spawn 后 AssignProcessToJobObject，drop 关闭 Job 句柄时整树终止（含子进程）；Job 创建失败静默降级为无作业托管。  4. **frp 热重载**（对应 rathole 热重载未实现项）：main.rs 新增 frp_reload(idx)——frp 隧道运行中执行 frpc reload -c <cfg> 不重启进程（热重载进程不托管 Job、detached 运行，避免 KILL_ON_JOB_CLOSE 竞争）；隧道页新增「热重载」按钮；rathole 无等价 reload 能力（仅 frp 支持）；
  5. **踩坑记录**：edit_file 在 CRLF 文件上多行插入「成功但未落盘」（mod logdb 与 player_prop 字段两次假成功，player_prop 还被误插入 ServerRuntime struct），改用 python_executor 统一换行后整段定位替换最稳，落盘后须复核锚点计数；egui 0.29.1 用 Frame::new() / CornerRadius（0.31 API），应使用 Frame::none() / Rounding::same(8.0)，Margin::same 参数为 f32；rusqlite 的 query_map match 两分支 params! 类型不同报 E0308，改用具名绑定 + collect 成 Vec 后再 extend 隔离生命周期。
- 2026-09-25（图标全面替换为「纯黑底 + 白色小写 e」新图案：源 dist\_embedded_512.png（512x512 黑色圆角底 + 居中白色小写 e），cargo build --release 通过（13.95s，6 warnings 0 error），dist 两份 exe 已重建覆盖（13:52，16,285,511 字节，RT_ICON(3)/RT_GROUP_ICON(14)/manifest(24) 齐全，ExtractAssociatedIcon 提取 32x32 与 assets\icon_32.rgba 逐像素 0 差异）：
  1. **三处图标统一替换（源码零改动）**：assets\icon_64.rgba（64x64 raw RGBA）与 assets\icon_32.rgba（32x32 raw RGBA）用 dist\_embedded_512.png LANCZOS 缩放重新生成并覆盖（窗口/托盘图标用 make_theme_icon_rgba include_bytes! 直接读取，无需改代码）；assets\xmst.ico 重新生成为新图案多尺寸 ICO（16/24/32/48/64/128/256，全部 32bpp，PIL 生成），由 build.rs embed-resource 嵌入 exe；窗口/托盘/exe 三处图标一致为黑底白 e。  2. **构建与发布**：cargo build --release 后台化执行（PID 23396），13.95s 完成（6 warnings 0 error）；dist 两份 exe（xmst.exe / 修暝的服务器工具.exe）已覆盖更新，SHA/大小一致（16,285,511 字节）；覆盖前「修暝的服务器工具.exe」被运行中进程占用（PID 19500），经用户确认后结束进程完成覆盖。  3. **验证**：pefile 资源看 RT_ICON(3)/RT_GROUP_ICON(14)/manifest(24) 齐全；System.Drawing.Icon.ExtractAssociatedIcon 提取 32x32 图标与 assets\icon_32.rgba 逐像素对比 0 差异，确认新图标已真实嵌入；另 assets\xmst_icon_256.png（egui 橙底 e 源图）与 dist\_icon_check*.png 保留未删- 2026-09-25（图标回退 egui 默认 e 图标：弃用「修暝」主题图标，cargo build --release 通过（6 warnings 0 error），dist 两份 exe 已重建覆盖（12:19，16,260,469 字节，ExtractAssociatedIcon 验证 exe 图标已嵌入）：  1. **图标回退（用户要求「就用 egui 的图标」）**：删除 make_theme_icon_rgba 程序化绘制「修暝」主题（暗色太空渐变 + 交叉双刃 X + 金色新月 + 星光）的全部像素生成代码与 lerp_u8 插值函数；make_theme_icon_rgba 改为直接 include_bytes! 嵌入 assets\icon_64.rgba（窗口 64x64）与 assets\icon_32.rgba（托盘 32x32）；assets\xmst.ico 重新生成为 egui 官方风格图标（橙色圆角底 + 白色 e，多尺寸 16..256）并由 embed-resource 嵌入 exe；窗口/托盘/exe 三处图标现为同款 egui e 图标；assets\xmst_icon_256.png 旧源图保留未删；
  2. **保留项**：自动备份禁用完全隐藏（10:42 已发布）、版本号 v0.1alpha、embed-resource 构建链路均不受影响；
- 2026-09-25（自动备份禁用完全隐藏 + 首个正式图标「修暝」主题 + 版本号 v0.1alpha，cargo build --release 通过（6 warnings 0 error），dist 两份 exe 已重建覆盖（10:42，16,243,198 字节，ExtractAssociatedIcon 验证 exe 图标已嵌入）：  （本条完整内容缺失，待人工补充）
- 2026-09-26（阶段7：首轮实测 7 个 bug 修复，构建时间 13:34:21（1,951,232 字节，SHA256 60380853E274312ADC38E10BA7F0FB66A01865F399F7C5974AC5B39ECA2A54C5）：
  1. **exe 图标橙色**（用户点名）：build.rs 嵌入的 xmst.ico 用 `dist\_embedded_512.png` 深色底 e 的 ICO 覆盖 + 同源 rgba。  2. **PowerShell/conhost 闪现、内存 81-141MB**：网速采样每秒 spawn `powershell Get-Counter` 已删，采样改用 Rust GetIfTable；所有 powershell/cmd spawn 全部隐藏窗口；windows_subsystem="windows"；托盘压缩线程释放 CJK 字体。  3. **×关闭黑底挂死、托盘无法退出**：托盘「退出」走 WM_CLOSE 被 tray 分支拦截成隐藏，新增独立 `tray_quit_requested` 退出标志直接 request_exit；关闭帧立即隐藏 + 3s 后压缩；
  4. **网络下载开关重启后失效**：App::new 时 dl 硬编码 None，改为初始化按 config 恢复的开关状态构建；
  5. **隧道备注乱码 + 多数输入框失效**：config JSON 历史 GBK 脏数据按 UTF-8 读；弹窗每帧局部 clone 绑定 TextEdit 失效，新增 repair_gbk_mojibake/repair_cfg_mojibake 修复回写；隧道配置改 tunnel_edit_draft 持久草稿字段。  6. **三项功能藏测试区**：default_enabled=false 的 Beta 项：网络下载/插件/玩家转正式默认开，Beta 列表 6 项。  7. **左上角双标题**：顶部标题栏 + 导航各绘一个，已删除导航头部重复绘制。- 2026-09-26（阶段2-6：重构规划批量落地，BETA 转正 + 主题系统 + 向导/强停）：
  1. 网络下载（BETA_DOWNLOAD→正式）、插件管理（BETA_PLUGINS→正式，rhai 宿主 + 事件总线 + zip 热加载 + max_operations）、玩家管理（BETA_PLAYERS→正式，新 Tab + RCON/文件 JSON 双路径 + 请求序号防错位）。  2. 主题系统：日/夜切换、预设/自定义配色、平滑过渡动画（帧率无关指数插值）、背景图、圆角开关 + 缩放滑杆、SeaLantern 布局。  3. 新人向导（B2：目录检测 + 五步开服向导）、强停二次确认（B3：pid+影响确认 + 一次性 token 30s 过期）；
  4. Beta 列表剩 3 项：beta.server.rathole / beta.server.remote_backup / 第三项按 features.rs REGISTRY 实查。  5. 踩坑：egui 0.29 与 0.31+ API 差异（Rounding 与 CornerRadius、screen_rect 与 content_rect、Mesh::with_texture + add_rect_with_uv、load_image_bytes 单参、Window::open 借用冲突 E0500）；rhai 1.x 无 has_function、on_progress 返回 ImmutableString、set_max_operations 防死循环；新增依赖需 dlltool（gnu 工具链 PATH 缺失报 program not found）；serde default 函数必须存在且可见（E0425）。- 2026-09-21（日志链路修复：stdout 改 tail latest.log）：新增 `process::LogFileTail` 文件尾随读取器（按字节偏移增量读取、文件轮转 len<offset 归零、行尾未闭合 pending 拼接、`\r` 进度条清理、单帧 1MB 上限、pending 64KB 强制切行）；日志源从 stdout 管道改为 `logs/latest.log`（绕开 Java stdout 缓冲，关服日志不再丢失），stdout 管道保留兜底（启动初期或 MC 服务器文件通道失效）；连续 60 帧打不开文件判定通道无效永久回退 stdout；涉及 src/process.rs、src/main.rs（ServerRuntime.log_tail、start_server 初始化、App::update 拉取循环）；产物 14.26MB。- 2026-09-20（备份策略与改名）：备份策略改为「首次全量 + 后续增量」（镜像清单对比，全量基线丢失自动补全量），存于 `.mcsrv_backups\`，移除 max_size_mb 限制（旧配置自动忽略）；备份移入后台线程 + 限速 `throttle_mbps`（默认 30 MB/s）+ 低优先级（THREAD_PRIORITY_BELOW_NORMAL）；工具更名「修暝的服务器工具」（窗口标题/顶部/单实例互斥体 `Local\XiumingServerTool_SingleInstance`/exe 文件名），编译产物 xiuming-server-tool.exe 与部署名「修暝的服务器工具.exe」。- 阶段1（网络下载模块，历史会话）：服务端下载（Vanilla/Paper/Fabric/Forge/NeoForge，Paper 用 fill.papermc.io/v3）+ Modrinth 模组下载/安装；为阶段 8 模组社区界面（搜索/详情/翻译/分页）打下基础。- 阶段0（托盘性能专项，历史会话）：托盘 10 重绘 + 工作集压缩（EmptyWorkingSet）、CJK 字体懒加载、multisampling=0 / vsync、release profile 优化、log_buf mem::take。- 阶段 9/10：**无独立会话记录留存**——既有记录中阶段 8 之后直接为阶段 C / 11 / 12，编号 9/10 未在 HANDOVER 与交接文档中出现，无法重建；如后续会话有相关资料再补。
## 1. 项目概述
- **项目**：XMST（修暝的服务器工具），Minecraft 服务器运行管理工具，Rust + egui/eframe 0.29 桌面应用，Windows 单文件 exe（原名 MCServerManager，后更名 XMST /「修暝的服务器工具」）。
- **技术栈**：Rust stable + x86_64-pc-windows-gnu 工具链（MinGW 链接器，`-static -mwindows`），vendor/eframe-0.29.1 本地 vendored GUI 框架；build.rs 用 embed-resource 嵌入图标/manifest；无 git、无卷影副本，文件被覆盖后不可回滚（见截断原因分析）。
- **代码结构**：根目录 F:\XMST，src/ 共 13 个模块——main.rs（应用主逻辑/UI/运行时管理，已超万行）、config.rs、modrinth.rs（Modrinth API/翻译/分页）、rcon.rs、plugins.rs（rhai 插件）、logdb.rs（SQLite 日志）、backup.rs、perf.rs、theme.rs、process.rs（子进程/日志尾随）、download.rs、server_download.rs、features.rs（功能开关注册表）；dist/data/xmst_config.json 为核心配置，dist/ 产出 xmst.exe 与「修暝的服务器工具.exe」两份同名 exe（2026-09-29 起改为单 exe `dist\XMST-<版本号>.exe`，见最近变更记录）。
- **文档**：HANDOVER.md（本文档，跨会话交接总纲）、会话交接_2026-09-26.md（阶段 0-7 记录）、README.md、docs/TODO-frp平台API接入.md。

## 2. 交付基线（务必以此为准）
| 项目 | 说明 |
| dist 两份 exe（历史基线，至 2026-09-29 凌晨） | `F:\XMST\dist\xmst.exe` 与 `F:\XMST\dist\修暝的服务器工具.exe`（同名同内容，覆盖须逐份核对，SHA256 一致） |
| dist 单 exe（2026-09-29 起） | `F:\XMST\dist\XMST-<版本号>.exe`（单文件命名含版本号，不再双 exe；构建流程见最近变更记录） |
| 最近构建时间（2026-10-01 第六轮） | `dist\XMST-0.1.0-alpha.exe` 2026-10-01 20:58 构建，SHA256 `564B44F078860023BC5D5A83A7B2787DFF489A5D0F9962FA1C2A75FCF6C00E0F`（与 release 一致）；**此后 21:26–21:27 backdrop.rs/main.rs 微调已修复编译错误并通过 cargo check（0 error），release 待重建，源码与产物不一致，见最近变更记录第七轮** |
| 最近构建时间 | 2026-09-29 02:17（UI 重构六批收尾 release），15,176,704 字节，SHA256 52C62830… |
| 阶段11 基线 | 2026-09-27 11:55（2,006,912 字节，SHA256 933DC229E65F15A1B1A6232D9D7ED7D331E1D144C94717FD5B8F6B0C55511ECB） |
| 阶段8 基线 | 2026-09-27 12:46（1,968,512 字节，SHA256 8CF3B78D3078EFA8A48836277B0EAD17AC00D7B655576776A9834FF7914F7343） |
| 阶段C 基线 | 2026-09-26 21:51（1,912,192 字节，SHA256 C2BD6C45…） |
| 会话交接基线 | 2026-09-26 13:34:21（1,951,232 字节，SHA256 60380853E274312ADC38E10BA7F0FB66A01865F399F7C5974AC5B39ECA2A54C5） |
| 图标 | `assets\xmst.ico`（黑底白 e，多尺寸 16..256，embed-resource 嵌入）；`assets\icon_32.rgba` / `icon_64.rgba` 为同源运行时图标（include_bytes! 读取） |
| 工具链要求 | Rust stable（实测 1.98.1）+ x86_64-pc-windows-gnu + MinGW（cargo/config.toml 指定 linker） |
## 3. 已完成功能清单（按入口/模块）
| 构建产物 | `target/release/xmst.exe`，部署名两份 exe（2026-09-29 起部署名为单文件 `dist\XMST-<版本号>.exe`） |
- 托盘 10 重绘 + 工作集压缩（EmptyWorkingSet）、CJK 字体懒加载、multisampling=0 / vsync、release profile 优化、log_buf mem::take。
- 主题系统：日/夜切换 + 预设/自定义配色 + 平滑过渡动画（帧率无关指数插值，收敛即停 repaint）、背景图（透明色 + ESC 退出编辑）、圆角开关 + 缩放滑杆、SeaLantern 布局（导航管理/系统分组）。
### 3.2 服务器管理
- 添加/启动/停止服务器、JVM 参数、控制台输入与日志查看；强停二次确认（pid+影响确认 + 一次性 token 30s 过期）；新人向导（目录检测 + 五步开服向导）。
- 日志链路：2026-09-21 起日志源由 stdout 管道改为文件尾随 `logs/latest.log`（LogFileTail，处理轮转/行尾未闭合/进度条清理/单帧 1MB 上限），stdout 管道兜底（启动初期或 MC 服务器文件通道失效时）。
### 3.3 备份（2026-09-20 策略定型）
- 全量 + 增量备份（镜像清单对比仅打包变化文件，全量基线丢失自动补全量），存于服务器目录 `.mcsrv_backups\`；限速 `throttle_mbps`（默认 30 MB/s）+ 后台线程 + 低优先级；备份列表（日期/类型）/删除（确认）/加载回退（确认）。
### 3.4 网络下载（Modrinth，阶段 1 起 → 正式）
- 服务端下载（Vanilla/Paper/Fabric/Forge/NeoForge，Paper 用 fill.papermc.io/v3）+ Modrinth 模组下载/安装。
- 模组社区搜索（阶段 8 收尾）：搜索栏 hint/回车触发/标签下拉 ComboBox/加载器筛选/卡片竖排字母图标；详情页六项（版本选择+过滤、完整简介、简单翻译、跳转链接、更新日志、图标与译文样式）。
- 搜索结果分页 + 数量控件（阶段 12）：limit clamp 1..=20 / offset，total_hits 统计，分页条「上一页 / 第 x/total 页 / 下一页」。
### 3.5 插件管理（阶段 2-6 起 → 正式）
- rhai 宿主 + 事件总线（server_started/stopped/log_line/player_joined/left/backup_done）+ zip 热加载 + max_operations 防死循环；内置示范包 `data\plugins\xmst-frosted-glass-demo.zip`（毛玻璃，DWM 真亚克力，失败回退半透明）。
- 每插件独立配置（2026-09-29）：configs: Arc<Mutex<HashMap<插件名, HashMap<k,v>>>> + xmst_config_get/set 脚本 API + 卡片「配置」折叠区编辑；背景效果入口（毛玻璃/半透明/恢复默认按钮 + 毛玻璃暗度滑杆）位于每张插件卡片的「配置」区内（非插件页总设置），读写该插件 bg_style/bg_alpha 键，脚本经 xmst_set_bg 触发时携带插件名并按该插件 bg_alpha 应用暗度。
### 3.6 玩家管理（阶段 2-6 起 → 正式）
- 服务器详情页新 Tab（在线/白名单/封禁/OP），双路径操作（RCON/文件 JSON）+ 请求序号防 A/B 服错位；玩家级属性编辑（阶段 C：游戏模式单选 + OP 勾选，控制台命令下发）。
### 3.7 穿透
- frp 集中管理（单 frpc 进程 + frpc.toml 多隧道）+ 热重载（阶段 C：frpc reload -c <cfg> 不重启进程）；rathole 穿透内核（frpc 兼容保留，NOISE 可选，Beta 默认关）。
### 3.8 日志与数据（阶段 C）
- SQLite 日志落库 + 轮转（src/logdb.rs，DEFAULT_MAX_ROWS=50000，增量写入 + 按来源过滤 + 清空），设置页展示行数/最近 5 条/清空按钮。
### 3.9 测试中功能（Beta 列表，默认关）
- `beta.server.rathole`、`beta.server.remote_backup`（备份完成远程转存：本地/UNC fs 复制 或 WebDAV PUT + Basic，失败按次数重试）；第三项按 features.rs REGISTRY 实查（历史含 BETA_BACKUP / BETA_CRASH_ANALYSIS / BETA_TRAFFIC 等，以代码为准）。
## 4. 版本条件热点表
> 跨会话实测沉淀：各依赖/环境版本下的关键条件与差异，改代码前先对照。
| 项目 | 版本/条件 | 要点 |
| Rust 工具链 | stable（实测 1.98.1，2026-08-05）+ x86_64-pc-windows-gnu | .cargo/config.toml 指定 mingw linker（gcc `-static -mwindows`）；新增带 C 依赖的 crate 需 dlltool，PATH 缺失报 `program not found`（备选：F:\XMST\temp\w64devkit\w64devkit\bin 或 F:\XMST\temp\mingw810\Tools\mingw810_64\bin） |
| egui/eframe | 0.29.1（vendor/ 本地 vendored） | 0.29 与 0.31+ API 差异：`Rounding`（非 `CornerRadius`）、`Frame::none()`、`Margin::same` 参数为 f32、`screen_rect`（非 content_rect）、`Mesh::with_texture + add_rect_with_uv`、`load_image_bytes` 单参签名、`Window::open` 借用冲突 E0500 |
| rhai | 1.x | 无 `has_function`；`on_progress` 闭包返回 `ImmutableString`；`set_max_operations` 防死循环 |
| rusqlite | bundled | SQLite 日志落库（src/logdb.rs），`query_map` 两分支 params! 类型不同报 E0308，用具名绑定 + collect 成 Vec 隔离生命周期 |
| embed-resource | build.rs | 嵌入 assets\xmst.ico 与 manifest；RT_ICON(3)/RT_GROUP_ICON(14)/manifest(24) 齐全，ExtractAssociatedIcon 逐像素验证 |
| windows_subsystem | Cargo.toml `package.windows_subsystem` 是 unused manifest key 警告 | 实际由 .cargo/config.toml rustflags `-mwindows` 生效；勿再在 Cargo.toml 重复配置 |
| serde | 任意 | default 函数必须存在且可见（E0425）；改配置默认值需同步 `Default` impl 与 serde default 两处 |
| Minecraft 语义 | Fabric 1.21.11 + Carpet + ServerCore + spark（2026-09-23 实测） | 无同步 TPS/MSPT 命令（/tps /mspt 报 Unknown，非 Paper/vanilla 语义）；/spark tps 经 RCON 返回空包（perf 需 10s 异步报告）；TPS/MSPT 待改从日志文件侧取数（见 7.1） |
| rathole | v0.5.0 | 官方下载 `https://github.com/rapiz1/rathole/releases/download/v0.5.0/rathole-x86_64-pc-windows-msvc.zip`（镜像 ghproxy/ghfast.top 按序重试）；NOISE 公钥需用户手工配服务端 |
| frp | frpc reload 热重载 | `frpc reload -c <cfg>` 不重启进程；热重载进程 detached 运行不托管 Job，避免 KILL_ON_JOB_CLOSE 竞争 |
| 构建检查基线 | 历史各阶段 | 60-61 warnings 0 error（noop_method_call 等既有警告）；构建必须后台化（Start-Process + 轮询日志 Finished），前台长命令被 shell 截断误判成功 |
## 5. 已知问题与待复测
### 5.1 待复测/验证项（修复后需用户实测确认）
- 托盘态内存是否降到预期：前台 81.7MB，目标工作集 <15MB、Private Bytes <45MB（阶段7 已删 PowerShell 采样并释放 CJK 字体，窗口态降到 106.8MB 略降，托盘态待实测）。
- 托盘关闭/退出完整链路：缩托盘 → 打开 → ×关闭（应真正隐藏无黑底）、托盘菜单退出（应真退进程）。
- 输入框输入与隧道备注显示（GBK 脏数据修复后）。
- 转正功能默认入口与开关重启持久化。
- 阶段 12 搜索结果分页与数量控件、阶段 11 详情页版本竖排/加载器过滤、本次 ①翻译缓存复用 ②搜索卡片点击穿透 ③分页数量 bug 修复后需真机复测。
- 构建警告（非新引入）：process.rs:136 unused_mut、main.rs:143 backup_working 未读、main.rs:212 new_server_dir 未读、backup.rs:38 files 未读、backup.rs:657 dir_size 未用、process.rs:168 drain_to_string 未用；建议后续清理或 `#[allow(dead_code)]`。
- 日志文件通道首次激活时的 stdout 已显示行有短暂重复（约十几行），属可接受切换开销。
- 旧测试备份约 35GB（4 个 zip）位于 `D:\Desktop\[7.3] AllTheMod10\.mcsrv_backups`，超出回收站容量无法由工具移入回收站，需手动删除。
### 5.3 未完成规划项（详见 7）
- Spark 获取 MSPT/TPS 与卡顿源分析（7.1，先不制作）；
- frp 平台 API 接入（7.2，未开始，详见 docs\TODO-frp平台API接入.md）；
- rathole 服务端配置推送（当前仅客户端生成）；
- WebDAV 用 PUT+Basic 整文件上传，无 PROPFIND/MKCOL/断点续传/锁（边界已登记）。
## 7. 待办与方案
### 7.1 Spark 获取 MSPT/TPS 与卡顿源分析（用户明确先不制作，先修 RCON，RCON 改造已完成，此为此下一阶段）
- **目标**：状态页展示 TPS/MSPT 与卡顿源分析，不再依赖 RCON 同步查询。
- **背景实测（2026-09-23 晚，Fabric 1.21.11 + Carpet + ServerCore + spark）**：该环境无任何同步 RCON 的 TPS/MSPT 命令——/tps /mspt 报 Unknown（非 Paper/vanilla 语义）；/carpet 状态块不稳定且输出 4096 截断；/spark tps 经 RCON 返回空包（异常输出进日志）；/perf 需 10s 异步报告落盘 debug/；/tick rate 仅返回目标值 20.0。因此 RCON 同步查询方案在此环境走不通，**改为从日志文件侧取数**。
- **方案设想（未实现，需实测确认）**：
  1. 通过控制台发送 /spark tps（或 /spark ticker），Spark 结果异步打印到服务器日志（[Spark] ... TPS ... MSPT ... 类似格式，**具体格式需实测**），工具侧扩展日志解析：新增 TPS/MSPT 正则（形如 `TPS from last 5s, 1m, 5m, 15m:` 行），解析结果写入状态页 TPS/MSPT 展示；复用既有 log_buf 实时收集 + 启动完成检测（buf.contains("Done (")）机制，无需新线程；
  2. 卡顿源分析：Spark 的 /spark profiler 需人工交互 + 上传报告，自动化成本高；可先用 /spark health（内置实体/区块健康概览，同样异步日志输出）作为轻量卡顿源信息；深度 profiling 后续评估。
  3. 备选：安装 TabTPS 类 mod 提供 TPS/MSPT 同步接口（如 tabtps 的 /tabtps RCON 命令可同步返回）——若日志解析不可行则考虑；但用户倾向少装 mod，优先 Spark 方案。
- **实现注意**：Spark 输出走日志，解析侧需在 main.rs 新增 parse_spark_tps 类函数（参考既有 parse_rcon_snapshot 的多格式兼容思路，但输入源是 log_buf 而非 RCON 快照）；状态页 TPS/MSPT 区块目前是占位说明（"待接入 Spark 方案（开发中）"），接入后替换为真实数据；UI 已有 RCON 配置入口，Spark 方案无需用户开任何端口/密码。
- 详见 docs\TODO-frp平台API接入.md，与本次 RCON 改造无交集。
## 8. 试错经验速查（跨会话沉淀）
> 本节汇总本项目多轮改造中反复出现的坑，新会话开工前先扫一遍。
1. **Rust 工程改动三板斧**：改前先 cargo check（快）→ 改后 cargo check（通过）→ cargo build --release → Copy-Item 到 dist（xmst.exe + 修暝的服务器工具.exe 两份同名文件）。构建用 Start-Process cargo build --release 后台 + 轮询 build_err.txt；**不要**直接跑长命令（输出会被 shell 截断，只见 Compiling 不见 Finished）。
2. **大段删除**：优先用 Python 脚本按行号区间删除 + 锚点定位；删除后必须 read_text 核对边界，防残留半行（如函数尾 sn\n}）或多删闭合括号（}); / }），症状是 unexpected closing delimiter / unclosed delimiter。
3. **括号配平**：ui_status 等长函数含多个 ScrollArea/Frame 闭包，手工 edit_file 改缩进易错；报错后先用 read_text 看函数头尾，再补/删闭合。
4. **impl App 内辅助函数**：方法内调用用 Self::xxx(...)；插入辅助函数时注意缩进是否在 impl 块内。
5. **假崩溃排查**：子线程 panic 会触发全局 panic hook 弹窗但主进程不挂（RCON 时代经典误报）。看到崩溃弹窗先查 data\crash.log 的 backtrace 线程名与 panic 位置，区分主/子线程，再决定是否真崩溃。
6. **exe 被占用**：窗口隐藏（托盘）时进程仍在，需按 Path/名称 Stop-Process 再覆盖 dist。
7. **历史坑（勿回退）**：伪闭合字符串（中文引号/转义错位）、hline 参数顺序（x_range 在前）、menu_on_left_click 与 with_menu_on_left_click、托盘事件用全局 set_event_handler 回调（窗口隐藏后事件循环休眠轮询不执行，已改回调内直接 send_viewport_cmd + request_repaint，tray_handlers_set 防重复注册）。
8. **文档纪律**：只写当前真实行为，未实现的功能（如 Spark）不写成已实现；UI 文档与代码实现脱节是用户反馈重灾区。
9. **发布核对（2026-09-25 新增）**：改源码后构建并覆盖 dist 后，必须核对 dist 两份 exe 的 LastWriteTime **晚于**最近改动源码文件的时间；用户报"没变化"时先按源码修改时间 vs dist exe 时间判断是没编译进去还是没覆盖，不要先怀疑代码改错。
10. **egui 0.29 API 差异（阶段 2-6 沉淀）**：`Rounding`（非 0.31+ 的 `CornerRadius`）、`screen_rect`（非 content_rect）、`Mesh::with_texture + add_rect_with_uv`、`load_image_bytes` 单参签名、`Window::open` 借用冲突 E0500，0.31 写法不可直接搬。
11. **rhai 1.x 差异（阶段 2-6 沉淀）**：无 `has_function`；`on_progress` 闭包返回 `ImmutableString`；必须 `set_max_operations` 防死循环。
12. **edit_file CRLF 假成功（阶段 C 沉淀）**：在 CRLF 文件上多行插入可能「成功但未落盘」（mod logdb 与 player_prop 字段两次假成功，player_prop 还被误插入 ServerRuntime struct）；改用 python_executor 统一换行后整段定位替换最稳，落盘后须复核锚点计数；同文件多段改动串行执行避免并发写锁失败。
13. **GBK 脏数据（阶段 7 沉淀）**：config JSON 历史 GBK 脏数据按 UTF-8 读会乱码（隧道备注等），需 repair_gbk_mojibake/repair_cfg_mojibake 修复回写；弹窗输入框每帧局部 clone 绑定 TextEdit 会失效，用持久草稿字段。
14. **交互穿透判定（2026-09-27 新增）**：egui 整卡/整行 interact click 会吞掉子控件点击（搜索卡片点名称进详情、点「译」按钮也进详情）；修复用 `resp.interact_pointer_pos()` 取落点，`rect.contains(p)` 判断是否落在标题/按钮等非跳转区，区分 Detail/FavToggle/None 动作；同法适用于翻译按钮（点按钮不触发行跳转）。
15. **翻译缓存状态机（2026-09-27 新增）**：「翻译简介/隐藏翻译」按钮只切换隐藏标志、不清缓存；翻译结果存 `mod_translate_result`（含 Option<Result>），有缓存再次显示直接读缓存，不再调 `modrinth::translate_text`。
16. **分页/数量变更重搜（2026-09-27 新增）**：搜索参数（mod_page_size/过滤/分页）变更后必须立即触发重搜并复位 mod_page/mod_total_hits/offset；busy 期间变更置 pending 标志，idle 后补搜，防止列表只渲染出一个模组。
## 9. 综合交接与经验总档（2026-09-29 合并自 HANDOVER_综合交接与经验总档.md，全文保留，含过时点注释）

> 用途：汇总 XMST 项目全部历史交接文档（HANDOVER.md、README.md、会话交接_2026-09-26.md、docs/session-handover-2026-09-28.md、docs/ISSUES_2026-09-28_Spark.md、docs/TODO-frp平台API接入.md）与历次会话踩坑经验，作为后续开发的一站式交接与防错基准。
> 生成日期：2026-09-28
> 纪律：正文中文；一切交付/构建/覆盖登记以**磁盘实证（时间戳 + SHA256）**为准，禁止以"登记完成"替代实际核验；**所有开发文件严格限制在 F:\XMST 内**。

---

### 1. 项目概述与架构决策

### 1.1 项目是什么

- **XMST（修暝的服务器工具）**：Minecraft 服务器运行管理工具，Rust + egui/eframe 0.29.1（本地 vendored）桌面应用，Windows 单文件 exe，原名 MCServerManager。
- **技术栈**：Rust stable（实测 1.98.1）+ x86_64-pc-windows-gnu + MinGW 链接器（`.cargo/config.toml` 指定 `gcc -static -mwindows`）；build.rs 用 embed-resource 嵌入图标/manifest；无 git、无 VSS、无卷影副本——**文件被覆盖后不可回滚**。
- **代码结构**：根目录 `F:\XMST`，src/ 共 13 个模块：`main.rs`（应用主逻辑/UI/运行时管理，已超万行）、`config.rs`、`modrinth.rs`（Modrinth API/翻译/分页）、`rcon.rs`、`plugins.rs`（rhai 插件）、`logdb.rs`（SQLite 日志）、`backup.rs`、`perf.rs`、`theme.rs`、`process.rs`（子进程/日志尾随）、`download.rs`、`server_download.rs`、`features.rs`（功能开关注册表）、`spark_analysis.rs`（Spark 解析与分析 UI，797 行）。

### 1.2 关键架构决策（跨会话沉淀，勿回退）

| 决策点 | 决策内容 | 原因/背景 |
|---|---|---|
| 日志链路（2026-09-21） | 日志源从 stdout 管道改为**文件尾随** `logs/latest.log`（`process::LogFileTail`：字节偏移增量、文件轮转 len<offset 归零、行尾未闭合 pending 拼接、`\r` 进度条清理、单帧 1MB 上限、pending 64KB 强制切行）；stdout 管道保留兜底（启动初期/非 MC 服务器/通道失效） | Java stdout 在管道场景被缓冲，启动后日志停滞、关服日志丢失；诊断证据：`logs/latest.log` 完整写满 589 行确认根因 |
| 备份策略（2026-09-20） | 「首次全量 + 后续增量」（镜像清单对比仅打包变化文件，全量基线丢失自动补全量），存服务器目录 `.mcsrv_backups\`；限速 `throttle_mbps`（默认 30 MB/s）+ 后台线程 + 低优先级（THREAD_PRIORITY_BELOW_NORMAL）；移除 max_size_mb（旧配置自动忽略） | 降低备份性能消耗、防爆内存 |
| 双 exe 发布（2026-09-20 更名起，2026-09-29 已废弃） | dist 下同时维护 `xmst.exe` 与 `修暝的服务器工具.exe` 两份**同名同内容** exe，覆盖必须逐份核对 | 部署名兼容；曾发生"只覆盖一份"导致登记失实。**【2026-09-29 起改为单 exe：dist\XMST-<版本号>.exe，不再双 exe，见 HANDOVER 最近变更记录】** |
| 构建产物 | `cargo build --release` → `target/release/xmst.exe`（约 15MB），部署名两份 exe | 单 cargo 工程产出单 exe。**【2026-09-29 起部署名改为 dist\XMST-<版本号>.exe 单文件】** |
| 主题系统（阶段 2-6） | 日/夜切换 + 预设/自定义配色 + 平滑过渡动画（帧率无关指数插值、收敛即停 repaint）+ 背景图（透明度 + ESC 退出编辑）+ 圆角开关 + 缩放滑杆 + SeaLantern 布局 | 用户视觉诉求 |
| 强停/向导（阶段 2-6） | 新人向导（目录检测 + 五步开服向导）；强停二次确认（pid+影响确认 + 一次性 token 30s 过期） | 防误操作 |
| Spark 方案（2026-09-23 实测定型） | 不用 RCON 同步取 TPS/MSPT（Fabric 1.21.11 + Carpet + ServerCore + spark 环境无同步命令：/tps /mspt 报 Unknown、/spark tps 返空包、/perf 需 10s 异步），改为**日志侧解析**思路；实际落地为 Spark profiler 输出文件（.sparkprofile/.sparkhealth）gzip+protobuf 解析 | 阶段 17/18 已实现，依赖官方 proto + prost 编译 |
| 版本控制 | 无 git/VSS，全部手动管理：HANDOVER 只补不删 + 发布前备份 dist 旧 exe 至 dist\backup（保留最近 3 份） | 文件覆盖不可回滚的替代防线 |

---

### 2. 全部历史错误与经验教训（重点）

> 本节为**最高优先级**防错内容，新会话开工前必须整节扫一遍。

### 2.1 重大事故：2026-09-28 UI 更新改坏（本期重点复盘）

**事故背景**：批量将 `Color32::from_rgb(...)` 替换为 `self.fg(Color32::from_rgb(...))`（`fg()` 定义于 main.rs:1657，亮色模式调 `theme::light_adapt` 压暗文字），意图实现亮色模式可读性。

**三类编译错误与修复方法**：

| # | 错误 | 位置/规模 | 根因 | 修复方法 |
|---|---|---|---|---|
| 1 | **漏写右括号约 158 处**（行内 101 + 跨行 57），cargo 报 mismatched / unclosed delimiter | 全局批量替换点 | 批量文本替换只改内容、未同步括号配平（如 `Color32::from_rgb(r,g,b)` 替换后丢失闭合括号） | 逐处补右括号；用 `cargo check` 反复定位；**教训：批量替换后必须全文括号配平核查** |
| 2 | **E0502 借用冲突 4 处**：main.rs:8786 / 10347 / 12108 / 12676 | 4 处可变借用上下文 | `self.fg(&self)` 不可变借用与字段 `as_mut` 可变借用冲突 | 在 4 处可变借用作用域前定义局部闭包 `let fg = \|c\| if self.cfg.theme_mode=="light"{theme::light_adapt(c)}else{c};`（行 8783/10343/12104/12638），作用域内裸调 `fg(...)` 替代 `self.fg(...)` |
| 3 | **E0425 误伤 3 处**：10536 / 12772 / 12779（已回改） | 3 处裸 `fg` 标识符 | 批量替换把本就存在的裸 `fg` 局部变量误认/误改 | 回改为原写法；修复后以 `Select-String` 全量核查裸 fg 调用均在闭包作用域内（8787/10462/12177/12186/12680） |

**构建与发布实证**：
- debug 构建 3m12s 成功，release 构建 3m20s 成功；产物 `target\debug\xmst.exe`（333.95MB）、`target\release\xmst.exe`（15,149,056B）；无 error，仅 66 warnings。
- dist 双 exe 已更新为 15,149,056B / 20:59:53，SHA256 = `E11FDA2A75EBD1995B233289828B96A819FF74FA82C7467D1FDEE6DC34E958D0`，与 release 产物一致。
- 旧版已备份至 `dist\backup\`（带时间戳 20260928_210720）。

**事故预防措施（本总档新增硬性条款）**：
1. **批量替换前必须小范围试点**：先在 1-2 处手工改 → `cargo check` → 确认无误再全量；严禁一次性对超万行文件做全量文本替换。
2. **涉及 `self.fg(...)` 等借用型包装方法的替换**：先确认目标作用域是否存在可变借用；存在则按"闭包前置"模式（见 2.1）处理。
3. **替换后做三类核查**：括号配平（cargo 报 mismatched/unclosed delimiter 时逐段二分定位）；借用冲突（E0502 定位到行，改闭包）；标识符误伤（E0425，用 Select-String 全量检索裸调用点）。
4. **记录改动行号清单**（本次修复依赖的行号：闭包 8783/10343/12104/12638，调用 8787/10462/12177/12186/12680），便于回退与审查。

### 2.2 构建与发布类踩坑（高频）

| 坑 | 现象 | 正确做法 |
|---|---|---|
| **前台构建被 shell 截断** | 前台跑 `cargo build --release` 超 600s 被 shell 超时杀，只见 Compiling 不见 Finished，误判成功 | **构建必须后台化**：`Start-Process cargo build --release` 写日志 + 轮询日志出现 `Finished`；仅凭无 error 输出不能判定成功 |
| **登记失实** | HANDOVER 登记"两份 exe 均已覆盖"，实际仅覆盖一份（dist 内多个命名副本只覆盖了默认名） | 覆盖后必须逐份核对 dist 下**每个** exe 的 LastWriteTime 晚于源码改动时间且 SHA256 一致；HANDOVER 以磁盘实证为准 |
| **exe 被占用** | 窗口隐藏（托盘）时进程仍在，覆盖 dist 失败 | 覆盖前按 ExecutablePath/名称 `Stop-Process` 释放占用（托盘隐藏态进程仍在运行） |
| **改后未生效疑云** | 用户报"没变化" | 先对比 源码 LastWriteTime vs dist exe LastWriteTime，判断是没编译进去还是没覆盖，不要先怀疑代码改错 |

### 2.3 代码编辑类踩坑（高发）

| 坑 | 现象 | 正确做法 |
|---|---|---|
| **edit_file CRLF 假成功** | CRLF 文件多行插入返回"成功"但实际未落盘（mod logdb / player_prop 两次中招，player_prop 还被误插入 ServerRuntime struct） | 敏感改动用 python_executor 统一换行后整段定位替换；落盘后必须复核锚点计数；同文件多段改动串行执行防并发写锁失败 |
| **大段删除括号配平** | 删区间后残留半行（`sn\n}`）或多删闭合（`});`/`}`），症状 unexpected/unclosed delimiter | Python 按行号区间删除 + 锚点定位；删除后 read_text 核对边界 |
| **跨会话编辑大文件行号漂移** | 凭记忆的行号与 edit_file old_str 不匹配，替换失败 | 改前先 Select-String / read_text 确认实际行号与锚点，再动手 |
| **UI 长函数括号错位** | ui_status 等长函数含多 ScrollArea/Frame 闭包，手工改缩进易错 | 报错后先 read_text 看函数头尾，再补/删闭合 |

### 2.4 Rust/egui 0.29 语言与 API 陷阱（跨会话沉淀）

| 类别 | 要点 |
|---|---|
| egui 0.29 vs 0.31+ | `Rounding`（非 `CornerRadius`）、`Frame::none()`、`Margin::same` 参数为 **f32**（`12.0` 而非 `12`）、`screen_rect`（非 content_rect）、`Mesh::with_texture + add_rect_with_uv`、`load_image_bytes` 单参签名、`Window::open` 借用冲突 E0500、`ProgressBar::new` 参数为 f32（pct 是 f64 需 `(pct / 100.0) as f32`）、Frame 圆角用 `rounding()` |
| 借用冲突 | 循环内 `Vec::drain(..)` 迭代器持有可变借用时再 push 报 E0499（收集 pending Vec 再整体替换）；`hot_top` 用 `nodes.into_iter().take(20)` 会 move nodes 后段再 sort_by 报 E0382（改 `iter().take()` 克隆）；if let 解引用后在闭包内置 None 报 E0500（用局部标志、闭包外写回）；方法内 `Self::xxx(...)` 与 `sc=&self.cfg.servers[i]` 不可变借用冲突报 E0502（收集意图 `switch_server: Option<usize>` 循环外执行） |
| serde | default 函数必须存在且可见（E0425）；改配置默认值需**同步** `Default` impl 与 serde default 两处 |
| 新增依赖 | 带 C 依赖的 crate 需 dlltool（gnu 工具链），PATH 缺失报 `program not found`；备选 PATH：`F:\XMST\temp\w64devkit\w64devkit\bin` 或 `F:\XMST\temp\mingw810\Tools\mingw810_64\bin` |
| prost/proto | prost 生成代码以 OUT_DIR 为准：字段报错先查 `target\debug\build\xmst-*\out\spark.rs` 确认真实字段名（如 `timestamp_deltas_ms` 非 `timestamp_deltas`）；StackTraceNode 无嵌套 children（扁平树 + children_refs 下标引用）；勿凭记忆猜 |
| rhai 1.x | 无 `has_function`；`on_progress` 闭包返回 `ImmutableString`；必须 `set_max_operations` 防死循环 |
| rusqlite | `query_map` 两分支 params! 类型不同报 E0308：用具名绑定 + collect 成 Vec 再 extend 隔离生命周期 |
| 其它 Rust | `ZipFile::take` 是 `std::io::Read` 方法非关联函数，trait 不在作用域报 E0599，`read_to_end` 首参 `&mut Take<..>` 不能内联临时值需 let 绑定；`Visuals` 无 `weak_bg_color`（用 `faint_bg_color`）；`features::is_enabled` 首参必须传 `&self.cfg.features`；ServerConfig 字段为 `dir:PathBuf`/`java_path:Option<String>`/`mc_version:Option<String>`/`jvm_args:Option<String>`（无 extra_jvm_args）；结构体新增字段必须补齐 Default（E0063，如 `file_delete_confirm: Option<String>` 默认 None）；`.args` 数组要求同类型（URL 需 `let url_ref: &str = &url;`） |
| 运行时进程 | **严禁运行时高频 spawn powershell/cmd**（控制台闪现 + 内存暴涨）：网速采样曾每帧 spawn `Get-Counter` 致内存 81-141MB，改纯 Rust GetIfTable；8 处 powershell/cmd spawn 全部隐藏窗口；windows_subsystem 由 `.cargo/config.toml` rustflags `-mwindows` 生效（Cargo.toml 里配置是 unused manifest key 警告，勿重复配置） |

### 2.5 数据与状态类踩坑

| 坑 | 现象 | 正确做法 |
|---|---|---|
| GBK 脏数据 | config JSON 历史 GBK 脏数据按 UTF-8 读乱码（隧道备注等） | `repair_gbk_mojibake` / `repair_cfg_mojibake` 修复回写；弹窗输入框每帧局部 clone 绑定 TextEdit 失效，用持久草稿字段（tunnel_edit_draft） |
| 假崩溃 | 子线程 panic 触发全局 panic hook 弹窗但主进程不挂（RCON 时代经典误报） | 先查 `data\crash.log` 的 backtrace 线程名与 panic 位置，区分主/子线程再决定是否真崩溃 |
| 交互穿透 | egui 整卡/整行 interact click 吞掉子控件点击（搜索卡片点名称/点「译」进详情） | `resp.interact_pointer_pos()` 取落点 + `rect.contains(p)` 区分 Detail/FavToggle/None 动作 |
| 翻译缓存状态机 | 「翻译简介/隐藏翻译」不清缓存会重复调用 translate_text | 只切换 hidden 标志，有缓存直接读 `mod_translate_result` |
| 分页/数量重搜 | 搜索参数变更后不重搜，列表只渲染一个模组 | 参数变更即重搜并复位 mod_page/mod_total_hits/offset；busy 期间置 pending 标志 idle 后补搜 |
| 相对路径误用 | Spark 输出文件点击报 `os error 3`：scan 存相对子目录、点击未拼服务器绝对目录 | 扫描/点击时用 `sc.dir.join(full)` 补全绝对路径再解析 |

### 2.6 文档与流程类纪律

- HANDOVER.md 曾被**意外截断**（2026-09-27）：无 VSS、无 git、写入链路为全量重写（每次写入重新生成 AIGC 头），证据链指向生成/写入环节截断。**教训：交接总档每次更新后建议另存备份（.bak 带时间戳），不得依赖单一副本。**
- 文档纪律：**只写当前真实行为**，未实现的功能不写成已实现；UI 文档与代码实现脱节是用户反馈重灾区。
- 注释英文、正文中文；不新增 AI 自加说明性 UI 文本。
- 删除/覆盖类操作先与用户确认（本项目无版本控制，覆盖即不可回滚）。

---

### 3. 各会话/阶段关键交付与决策汇总

> 阶段编号按 HANDOVER 与交接文档登记；阶段 9/10 无记录留存（如实标注）。

| 阶段/日期 | 关键交付与决策 |
|---|---|
| 阶段 0（历史） | 托盘性能专项：托盘 0 重绘（10 重绘全禁）+ 工作集压缩 EmptyWorkingSet、CJK 字体懒加载、multisampling=0 / vsync、release profile 优化、log_buf mem::take |
| 阶段 1（历史） | 网络下载模块：服务端下载（Vanilla/Paper/Fabric/Forge/NeoForge，Paper 走 fill.papermc.io/v3）+ Modrinth 模组下载/安装；为阶段 8 模组社区界面打基础 |
| 阶段 2-6（2026-09-26） | 重构规划批量落地：网络下载/插件管理（rhai 宿主 + 事件总线 server_started/stopped/log_line/player_joined/left/backup_done + zip 热加载 + max_operations）/玩家管理（新 Tab + RCON/文件 JSON 双路径 + 请求序号防 A/B 服错位）三功能 Beta→正式；主题系统（日/夜 + 预设/自定义 + 平滑过渡 + 背景图 + 圆角/缩放 + SeaLantern 布局）；新人向导（B2）；强停二次确认（B3）；Beta 列表剩 3 项 |
| 阶段 7（2026-09-26） | 首轮实测 7 项 bug 修复：exe 图标橙色（build.rs 旧 ico）、PowerShell/conhost 闪现+内存 81-141MB（删 Get-Counter、隐藏全部 spawn、windows_subsystem）、×关闭黑底挂死/托盘无法退出（独立 tray_quit_requested 标志）、网络下载开关重启失效（App::new 按 config 构建）、隧道备注乱码/输入框失效（GBK 修复 + 持久草稿）、三项功能藏测试区（转正式）、左上角双标题（删导航重复绘制）。基线：13:34:21，9,951,232B，SHA256 60380853… |
| 阶段 8（2026-09-27） | 模组社区界面收尾：创建窗口关不掉（open 参数兜底关闭）、搜索栏五项（hint/回车触发/标签 ComboBox/间距收紧/加载器筛选保留）、卡片竖排字母图标（painter.galley + layout_no_wrap 降字号）、详情页六项（版本选择过滤/完整简介/简单翻译 translate.googleapis.com gtx/跳转链接/更新日志 strip_html）。基线 12:46，1,968,512B |
| 阶段 C（2026-09-26） | 四功能一次性完成：玩家级属性编辑（gamemode/op 控制台命令下发）、SQLite 日志落库+轮转（logdb.rs，DEFAULT_MAX_ROWS=50000）、kill-on-drop 防孤儿（Job Object + JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE）、frp 热重载（frpc reload -c <cfg>，热重载进程 detached 不托管 Job）。基线 21:51，1,912,192B |
| 阶段 11（2026-09-27） | 详情页三项反馈：版本列表竖排、加载器动态过滤（loaders 并集按 fabric/forge/neoforge/quilt 固定序）、头部按钮左移/版本行去安装按钮/预览页收两按钮（dl_mod_open_page 用 cmd start）。基线 11:55，2,006,912B |
| 阶段 12（2026-09-27） | 图1图2三项 UI 反馈：译文位置与样式（按钮上方绿色字）、列表加模组图标（mod_icon_ui）、搜索结果分页 + 数量控件（limit 1..=20/offset/total_hits/分页条）。dist\xmst.exe 已重建；「修暝的服务器工具.exe」因被占用未覆盖（PID 11184）——**教训见 2.2** |
| 阶段 13（2026-09-27） | 四项反馈修复 + HANDOVER.md 重建：翻译缓存复用（!mod_translate_hidden 判断）、搜索卡片点击穿透（interact_pointer_pos 落点判定）、分页数量 bug（pending 排队）、HANDOVER 补齐截断分析与章节 1-5/阶段 0-10/试错速查 10-16。基线 19:50，4,676,992B |
| 阶段 14（2026-09-27） | 文件浏览页「排查客户端模组」：jar_is_client_mod 静态解析（fabric.mod.json environment=="client"；mods.toml/neoforge.mods.toml [[mods]] side=="CLIENT"）+ 二次确认 + 标橙置顶 + 状态不持久化（clear_client_mod_marks）。基线 20:27，4,840,320B |
| 阶段 15（2026-09-27） | 模组社区下载四项反馈：进度条移至下载区下方、详情页不退 + datapack 归类改 world/datapacks、搜索版本置顶高亮（mod_detail_mc 橙色）、加载器筛选自动跟随。基线 21:12，4,851,584B |
| 阶段 16（2026-09-27） | 四个界面问题修复：左侧分类点击收起详情页、快照版本隐藏 + 显示开关（mod_show_snapshot）、下载进度条缩短 30%（available_width*0.70 封顶 160px）、文件浏览删除进回收站 + 二次确认（delete_to_recycle_bin 走 Microsoft.VisualBasic.FileIO + SendToRecycleBin，禁止永久删除）。基线 21:54，4,862,848B |
| 阶段 17（2026-09-27） | 特殊功能-Spark 框架：ServerTab::Special 页签、BETA_SPECIAL/FEATURE_SPARK 开关（REGISTRY order 90/91，默认关）、detect_spark（mods\*.jar 名含 spark）、scan_spark_files（config\spark + plugins\spark 下 .sparkprofile/.sparkhealth）、proto 依赖（flate2/prost/protoc-bin-vendored，官方 spark.proto 6338B + spark_sampler.proto 2410B）。基线 22:34，4,889,472B |
| 阶段 18（2026-09-27） | Spark 解析与分析 UI：build.rs 设 PROTOC vendored 后 compile_protos（OUT_DIR 扁平 spark.rs）；parse_spark_file（gzip 魔数 1f 8b 判断解压 + protobuf 解码，失败 String 回传不 panic）；分析 UI 概览卡阈值着色/TPS-MSPT 波动曲线/模组性能 TOP/卡顿热点 TOP。基线 SHA256 575CD5AD… |
| 2026-09-28 | Spark 三问题修复：①输出文件点击 os error 3（sc.dir.join(full) 补绝对路径）；②分析 UI 排版重构（总折叠 + 子折叠 + 左 300px 控制/右预览，tick_spark_prof 到期自动 stop --save-to-file）；③特殊功能转正式（BETA_SPECIAL/FEATURE_SPARK group Beta→Server、default_enabled true、去 beta_confirm 弹窗） |
| 2026-09-28 | Spark 反馈第二轮：概览改可折叠展开（ui_metric_collapse + 中文判定 0/1/2 级）、模组定位打分制（score_match：3 全等/2 来源前缀/1 候选前缀/0 双向包含，别名优先）、萌新可读（去 striped 交替背景）、emoji 缺字形改纯文字；SparkSummary 新增 heap/cpu/entity 序列 |
| 2026-09-28 | 三项 UI 反馈：Spark 概览超界修复（弃 Grid 改手动分行按 available_width 算列数封顶 340px）、窗口自定义缩放（with_resizable(true)）、左侧栏平滑折叠（servers_collapsed/servers_anim，240px↔44px，折叠图标化） |
| 2026-09-28 | 分析结果页页签化（SparkViewTab：概览/波动曲线/模组性能/卡顿热点）+ 卡顿源→模组定位（match_mod_jar 元数据别名）+ 跳转文件浏览（mod_jar_index 惰性索引 + file_scroll_to）+ 混淆映射说明（class_10209=Profilers，method_64148=deactivate()V） |
| 2026-09-28 | 亮色模式对比度修复 + 另一 AI 修改建议评估：弱文字亮色置 from_gray(120)（theme.rs apply）、预设配色亮色仅取压暗强调色（target_colors）、hyperlink/选区/光标统一 light_adapt(accent)、导航栏文字 205→170；评估结论：对比度类采纳，字号/宽度/结构重构类暂不采纳。dist 双 exe SHA256 0B24CD50… |
| 2026-09-28 | **UI 更新改坏事故与修复（见 2.1）**：批量 self.fg 替换导致 158 处漏括号 + E0502×4 + E0425×3，修复后 dist 双 exe SHA256 E11FDA2A… |
| 2026-09-29 | UI 重构六批收尾（修改建议 10 项全落地）：主题系统重构（删预设只留日/夜 + 太阳月亮切换 + theme_follow_system 默认跟随）、强调色 #22D3EE、ui_font_scale 14（11-20）、全页 1100px 居中、滚动条增强、导航二级缩进、顶部栏三区、设置三页、插件卡片、下载页三栏、Spark 页改进；六批 cargo check 0 error；release 3m14s Finished（71 warnings）；dist 双 exe 覆盖 15,176,704 B SHA256 52C62830…（旧 E11FDA2A… 已备份 dist\backup\*_20260929_022049.bak） |

---

### 4. 文件存储位置规范（硬性约束）

### 4.1 总原则

- **项目唯一根目录：`F:\XMST`。所有开发文件（源码、文档、构建产物、发布产物、备份、中间产物）一律存储在 F:\XMST 内，严禁写到其它位置（桌面、C:\temp、D:\Desktop、系统临时目录等）。**
- 除 F:\XMST 外，仅允许的例外：用户明确指定的服务器数据目录（MC 服务器本体及其 .mcsrv_backups 备份）、`F:\修暝的神秘提示词\`（prompt 系列文档，只读参考）。
- 中间产物必须写入项目 temp 目录；最终产出写入用户指定的 F:\XMST 内目标路径。

### 4.2 目录用途表

| 目录 | 用途 | 说明/注意事项 |
|---|---|---|
| `F:\XMST\src\` | 全部 Rust 源码模块 | main.rs 已超万行；改前先定位锚点 |
| `F:\XMST\src\spark_proto\spark\` | Spark 官方 proto（spark.proto / spark_sampler.proto） | build.rs 编译，OUT_DIR 扁平 spark.rs |
| `F:\XMST\assets\` | 图标资源：`xmst.ico`（多尺寸 16..256 黑底白 e，embed-resource 嵌入）、`icon_32.rgba` / `icon_64.rgba`（运行时 include_bytes!）、`_embedded_512.png` 源图 | 与 exe 三处图标一致 |
| `F:\XMST\dist\` | **发布产物目录**：`xmst.exe` + `修暝的服务器工具.exe`（双 exe 同名同内容）+ `data\xmst_config.json` 核心配置 | 覆盖须逐份核对 SHA256；覆盖前 Stop-Process 释放占用。**【2026-09-29 起单 exe：dist\XMST-<版本号>.exe】** |
| `F:\XMST\dist\backup\` | 发布前旧版 exe 备份（带时间戳 .bak） | 保留最近 3 份策略 |
| `F:\XMST\docs\` | 交接文档、问题记录、待办：`session-handover-2026-09-28.md`、`ISSUES_2026-09-28_Spark.md`、`TODO-frp平台API接入.md` | 会话交接与问题记录统一放此 |
| `F:\XMST\temp\` | 工具链（w64devkit / mingw810）、中间产物、构建日志 | 可重建；勿误删工具链 bin（dlltool 备选 PATH） |
| `F:\XMST\target\` | cargo 构建产物（debug/release/build） | 可重建；`target\release\xmst.exe` 为发布源 |
| `F:\XMST\vendor\` | vendored 依赖（eframe-0.29.1 等） | 勿动 |
| `F:\XMST\data\` | 运行时数据（plugins 示范包、logs 等） | — |
| `F:\XMST\HANDOVER.md` 等根目录文档 | 交接总档、README、综合总档（本文档） | HANDOVER 只补不删；更新后另存 .bak |
| `F:\XMST\.cargo\config.toml` | 工具链配置（mingw linker `gcc -static -mwindows`） | 勿改，改则构建失败 |

### 4.3 对外部路径的约束

- **禁止**将项目产物写入：桌面、`C:\temp`、`C:\Windows`、系统临时目录（$env:TEMP 仅用于工具链自身）、其它盘根目录。
- 服务器数据（MC 服务器目录、`.mcsrv_backups\` 备份）由用户指定位置管理，工具读写仅限服务器配置的 dir；历史遗留 35GB 旧备份位于 `D:\Desktop\[7.3] AllTheMod10\.mcsrv_backups`（超回收站容量，需手动删除）。

---

### 5. 后续开发防错清单

> 每次改动前逐项过；全部 ✅ 才可进入构建发布。

### 5.1 构建发布流程（每次改动后必走）

1. **cargo check 先行**：`cargo check`（约 9-40s，可前台）验证编译，0 error 后再继续。
2. **cargo build --release 必须后台化**：`Start-Process cargo build --release` 写日志 + 轮询日志直到 `Finished`（约 3m-4m30s）；**严禁前台跑**（被 shell 截断误判成功）。必要时 PATH 注入 `F:\XMST\temp\w64devkit\w64devkit\bin` 或 `F:\XMST\temp\mingw810\Tools\mingw810_64\bin`。
3. **覆盖 dist 双 exe**：`target\release\xmst.exe` → `dist\xmst.exe` **和** `dist\修暝的服务器工具.exe` **两份都覆盖**；覆盖前按路径 Stop-Process 释放占用；旧版先复制到 `dist\backup\`（带时间戳）。**【2026-09-29 起改为单 exe：`target\release\xmst.exe` → `dist\XMST-<版本号>.exe`，不再双 exe】**
4. **SHA256 核对**：`Get-FileHash` 校验两份 exe 哈希一致，且 LastWriteTime 晚于最近改动源码时间。
5. **更新 HANDOVER.md**：顶部「最近变更记录」倒序插入条目，写明阶段号、构建耗时、文件大小、SHA256 前 16 位；只补不删，字节级定点插入；更新后另存 .bak 备份。
6. **记录时间戳**：构建完成时间（本地时间），供回溯。

### 5.2 代码修改规范

1. **改前锚点**：跨会话编辑大文件先 Select-String / read_text 确认实际行号与锚点，不凭记忆。
2. **批量替换必须试点**：先在 1-2 处手工改 → cargo check → 全量；全量后做括号配平 / 借用冲突 / 标识符误伤三类核查（见 2.1 预防措施）。
3. **借用规则**：可变借用作用域内禁调 `self.fg(&self)` 等不可变借用方法——用闭包前置模式；闭包内改动状态用局部标志、循环外写回；借用冲突收集意图变量再执行。
4. **结构体字段**：新增字段必须同步 Default 初始化（E0063），`Option` 字段默认 None。
5. **配置默认值**：改默认值同步 `Default` impl 与 serde default 两处（E0425）。
6. **CRLF 文件**：敏感改动用 python_executor 字节级写入；落盘后复核锚点计数。
7. **禁止运行时高频 spawn**：任何 powershell/cmd 调用必须隐藏窗口；高频采样用纯 Rust 实现。

### 5.3 交接文档维护规范

1. HANDOVER.md 为跨会话交接总纲：**只补不删**；最近变更记录倒序插入顶部。
2. 登记以**磁盘实证**为准（时间戳/SHA256/字节数），禁止写未验证的"均已覆盖"。
3. 文档只写当前真实行为，未实现功能不写成已实现。
4. 会话结束生成 `docs\session-handover-日期.md`（完成项/待办/已知问题/关键文件位置/构建流程）。
5. 新踩坑即时追加到本文档「2. 全部历史错误与经验教训」对应小节 + HANDOVER「试错经验速查」。

### 5.4 UI 改动注意事项

1. egui 0.29 API 核对：Rounding/Frame::none()/Margin f32/screen_rect/load_image_bytes 单参/rounding()（0.31 写法不可直接搬）。
2. UI 长函数（含多 ScrollArea/Frame 闭包）改前读头尾，改后括号配平核查。
3. 交互穿透：整卡点击用 interact_pointer_pos + rect.contains 区分子控件动作。
4. 进度条/数值：ProgressBar::new 参数 f32；emoji 在 egui 默认字体可能缺字形（改纯文字）。
5. 布局自适应：Grid 长文本会撑宽超界——按 available_width 手动分行/算列数。
6. 折叠/动画：沿用 nav_anim 指数逼近风格；ui_animations 关闭时直接切换。
7. 改 UI 后对照主题系统：亮色模式需 light_adapt 压暗文字，弱文字亮色 from_gray(120)，预设配色亮色仅取压暗强调色。

---

### 6. 当前已知问题与待办

### 6.1 编译警告（非致命，建议清理）

- 当前基线 66 warnings：`egui::Stroke::new` 浮点 fallback 未来变 hard error（main.rs:5376/5383/5404/5422、spark_analysis.rs:1483/1492/1523）；未用导入（download.rs:13、main.rs:4467/4753/14860）；多余 mut 多处；build.rs:19 unused_must_use；theme.rs:101 多余括号。
- 历史既有：process.rs:136 unused_mut、main.rs:143 backup_working 未读、main.rs:212 new_server_dir 未读、backup.rs:38 files 未读、backup.rs:657 dir_size 未用、process.rs:168 drain_to_string 未用。

### 6.2 待复测/验证

- 托盘态内存目标：工作集 <15MB、Private Bytes <45MB（阶段 7 修复后待实测）。
- 托盘关闭/退出完整链路、输入框与隧道备注 GBK 修复后显示、转正功能开关重启持久化。
- 阶段 11-16 各 UI 反馈修复后真机复测（详情页版本竖排/加载器过滤、翻译缓存、点击穿透、分页数量、排查客户端模组等）。
- Spark 分析：真实服务器导出文件验证（含中文字符/异常数据/大文件）；Spark 输出文件保存目录补扫描路径（非 config\spark / plugins\spark 时）。

### 6.3 未完成规划项

- frp 平台 API 接入（仅调研准备，详见 docs\TODO-frp平台API接入.md）：frps 侧在线状态/连接数/流量展示；不引入非必要运行时依赖；API Token 按敏感字段处理。
- rathole 服务端配置推送（当前仅客户端生成，NOISE 公钥需手工配服务端）。
- WebDAV 远程备份仅 PUT+Basic 整文件上传，无 PROPFIND/MKCOL/断点续传/锁。
- pmr HTTP API 对接；服务器级独立环境变量支持。
- Spark 日志侧 TPS/MSPT 实时解析（原 7.1 思路，需实测 /spark tps 日志格式）。
- Beta 列表余项：beta.server.rathole、beta.server.remote_backup（第三项以 features.rs REGISTRY 实查）。

### 6.4 数据/文档遗留

- 旧测试备份约 35GB（14 个 zip）位于 `D:\Desktop\[7.3] AllTheMod10\.mcsrv_backups`，超回收站容量需手动删除。
- HANDOVER.md 早期章节缺失待人工补充：阶段 9/10 无记录；「2026-09-25 自动备份禁用完全隐藏」条目内容缺失。
- 日志文件通道首次激活与 stdout 已显示行有短暂重复（约十几行，可接受；彻底消除需启动即禁用 stdout 显示，代价丢 run.bat echo）。

---

---

## 附录. 重建补全说明（2026-09-27）
- 本版在保留原 HANDOVER.md 全部未损坏内容（最近变更记录阶段 12/11/8/C/图标、章节 7/8、试错速查 1-9）基础上，补全：截断原因分析、章节 1（项目概述）、章节 2（交付基线）、章节 3（已完成功能清单）、章节 4（版本条件热点表）、章节 5（已知问题与待复测）、阶段 0-10 变更记录、试错速查 10-16。
- 来源：会话交接_2026-09-26.md（阶段 0-7）、README.md（2026-09-20/21）、docs\TODO-frp平台API接入.md、HANDOVER.md 残留尾部与历次会话记录。
- 阶段 9/10 无任何记录佐证，如实标注缺失，未臆造；「2026-09-25 自动备份禁用完全隐藏」条目完整内容亦缺失，待人工补充。
*（内容由AI生成，仅供参考）*