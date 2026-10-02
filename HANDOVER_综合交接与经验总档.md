---
AIGC:
    Label: "1"
    ContentProducer: 001191440300708461136T1XGW3
    ProduceID: f60842f40542d18996ccf15309f5e3de_4bcbb34abb4011f1b172525400248c00
    ReservedCode1: nLB06+MLSvGu3Xp+tynQjZFKvYrHOPq6NRSVzVYYKEfFjFQUjjKgtYiQWK6TMFC1vIp89jG1+V8iXCJrVPsvWcwqb1dibqhYqjsfrI1zBfr1eYMK66addD7+tP6L7mAjoU6KK+qYSDgp9/XJqaPwJenjdW5ME2lO2zqsCvfmBy2YVEjgxnCzJyjxiLg=
    ContentPropagator: 001191440300708461136T1XGW3
    PropagateID: f60842f40542d18996ccf15309f5e3de_4bcbb34abb4011f1b172525400248c00
    ReservedCode2: nLB06+MLSvGu3Xp+tynQjZFKvYrHOPq6NRSVzVYYKEfFjFQUjjKgtYiQWK6TMFC1vIp89jG1+V8iXCJrVPsvWcwqb1dibqhYqjsfrI1zBfr1eYMK66addD7+tP6L7mAjoU6KK+qYSDgp9/XJqaPwJenjdW5ME2lO2zqsCvfmBy2YVEjgxnCzJyjxiLg=
---

> **【已归档 2026-09-29】** 本文件全部内容已并入 HANDOVER.md「第 9 章 综合交接与经验总档」，此后仅维护 HANDOVER.md；本文件退役保留备查，不再更新。

# XMST 综合交接与经验总档

> 用途：汇总 XMST 项目全部历史交接文档（HANDOVER.md、README.md、会话交接_2026-09-26.md、docs/session-handover-2026-09-28.md、docs/ISSUES_2026-09-28_Spark.md、docs/TODO-frp平台API接入.md）与历次会话踩坑经验，作为后续开发的一站式交接与防错基准。
> 生成日期：2026-09-28
> 纪律：正文中文；一切交付/构建/覆盖登记以**磁盘实证（时间戳 + SHA256）**为准，禁止以"登记完成"替代实际核验；**所有开发文件严格限制在 F:\XMST 内**。

---

## 1. 项目概述与架构决策

### 1.1 项目是什么

- **XMST（修暝的服务器工具）**：Minecraft 服务器运行管理工具，Rust + egui/eframe 0.29.1（本地 vendored）桌面应用，Windows 单文件 exe，原名 MCServerManager。
- **技术栈**：Rust stable（实测 1.98.1）+ x86_64-pc-windows-gnu + MinGW 链接器（`.cargo/config.toml` 指定 `gcc -static -mwindows`）；build.rs 用 embed-resource 嵌入图标/manifest；无 git、无 VSS、无卷影副本——**文件被覆盖后不可回滚**。
- **代码结构**：根目录 `F:\XMST`，src/ 共 13 个模块：`main.rs`（应用主逻辑/UI/运行时管理，已超万行）、`config.rs`、`modrinth.rs`（Modrinth API/翻译/分页）、`rcon.rs`、`plugins.rs`（rhai 插件）、`logdb.rs`（SQLite 日志）、`backup.rs`、`perf.rs`、`theme.rs`、`process.rs`（子进程/日志尾随）、`download.rs`、`server_download.rs`、`features.rs`（功能开关注册表）、`spark_analysis.rs`（Spark 解析与分析 UI，797 行）。

### 1.2 关键架构决策（跨会话沉淀，勿回退）

| 决策点 | 决策内容 | 原因/背景 |
|---|---|---|
| 日志链路（2026-09-21） | 日志源从 stdout 管道改为**文件尾随** `logs/latest.log`（`process::LogFileTail`：字节偏移增量、文件轮转 len<offset 归零、行尾未闭合 pending 拼接、`\r` 进度条清理、单帧 1MB 上限、pending 64KB 强制切行）；stdout 管道保留兜底（启动初期/非 MC 服务器/通道失效） | Java stdout 在管道场景被缓冲，启动后日志停滞、关服日志丢失；诊断证据：`logs/latest.log` 完整写满 589 行确认根因 |
| 备份策略（2026-09-20） | 「首次全量 + 后续增量」（镜像清单对比仅打包变化文件，全量基线丢失自动补全量），存服务器目录 `.mcsrv_backups\`；限速 `throttle_mbps`（默认 30 MB/s）+ 后台线程 + 低优先级（THREAD_PRIORITY_BELOW_NORMAL）；移除 max_size_mb（旧配置自动忽略） | 降低备份性能消耗、防爆内存 |
| 双 exe 发布（2026-09-20 更名起） | dist 下同时维护 `xmst.exe` 与 `修暝的服务器工具.exe` 两份**同名同内容** exe，覆盖必须逐份核对 | 部署名兼容；曾发生"只覆盖一份"导致登记失实 |
| 构建产物 | `cargo build --release` → `target/release/xmst.exe`（约 15MB），部署名两份 exe | 单 cargo 工程产出单 exe |
| 主题系统（阶段 2-6） | 日/夜切换 + 预设/自定义配色 + 平滑过渡动画（帧率无关指数插值、收敛即停 repaint）+ 背景图（透明度 + ESC 退出编辑）+ 圆角开关 + 缩放滑杆 + SeaLantern 布局 | 用户视觉诉求 |
| 强停/向导（阶段 2-6） | 新人向导（目录检测 + 五步开服向导）；强停二次确认（pid+影响确认 + 一次性 token 30s 过期） | 防误操作 |
| Spark 方案（2026-09-23 实测定型） | 不用 RCON 同步取 TPS/MSPT（Fabric 1.21.11 + Carpet + ServerCore + spark 环境无同步命令：/tps /mspt 报 Unknown、/spark tps 返空包、/perf 需 10s 异步），改为**日志侧解析**思路；实际落地为 Spark profiler 输出文件（.sparkprofile/.sparkhealth）gzip+protobuf 解析 | 阶段 17/18 已实现，依赖官方 proto + prost 编译 |
| 版本控制 | 无 git/VSS，全部手动管理：HANDOVER 只补不删 + 发布前备份 dist 旧 exe 至 dist\backup（保留最近 3 份） | 文件覆盖不可回滚的替代防线 |

---

## 2. 全部历史错误与经验教训（重点）

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

## 3. 各会话/阶段关键交付与决策汇总

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

## 4. 文件存储位置规范（硬性约束）

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
| `F:\XMST\dist\` | **发布产物目录**：`xmst.exe` + `修暝的服务器工具.exe`（双 exe 同名同内容）+ `data\xmst_config.json` 核心配置 | 覆盖须逐份核对 SHA256；覆盖前 Stop-Process 释放占用 |
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

## 5. 后续开发防错清单

> 每次改动前逐项过；全部 ✅ 才可进入构建发布。

### 5.1 构建发布流程（每次改动后必走）

1. **cargo check 先行**：`cargo check`（约 9-40s，可前台）验证编译，0 error 后再继续。
2. **cargo build --release 必须后台化**：`Start-Process cargo build --release` 写日志 + 轮询日志直到 `Finished`（约 3m-4m30s）；**严禁前台跑**（被 shell 截断误判成功）。必要时 PATH 注入 `F:\XMST\temp\w64devkit\w64devkit\bin` 或 `F:\XMST\temp\mingw810\Tools\mingw810_64\bin`。
3. **覆盖 dist 双 exe**：`target\release\xmst.exe` → `dist\xmst.exe` **和** `dist\修暝的服务器工具.exe` **两份都覆盖**；覆盖前按路径 Stop-Process 释放占用；旧版先复制到 `dist\backup\`（带时间戳）。
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

## 6. 当前已知问题与待办

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

*本文档由 file-agent 于 2026-09-28 汇总生成，基于 HANDOVER.md、README.md、会话交接_2026-09-26.md、docs/session-handover-2026-09-28.md、docs/ISSUES_2026-09-28_Spark.md、docs/TODO-frp平台API接入.md 及历次会话记录；所有构建/覆盖数据以磁盘实证为准。*
*（内容由AI生成，仅供参考）*
*（内容由AI生成，仅供参考）*
