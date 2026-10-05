# XMST 项目指南（索引 · 经验 · 排查手册）

> 面向接手本项目的 AI / 开发者。**只保留"去哪找什么"和"踩过的坑"**，不再记录逐轮改动流水账（历史见 git log）。
> 最近一次大修：2026-10-04（低完整性目录第二次 + 0.1.1-alpha 收尾批，见 §4、§6、§9）。

---

## 1. 这是什么

Rust 桌面端 Minecraft 服务器管理工具：**egui/eframe 0.29 + glow(OpenGL) + winit 0.30 + glutin**，Windows 专用。

| 事实 | 值 |
|---|---|
| 构建目标 | `x86_64-pc-windows-gnu`（宿主即 mingw） |
| 链接器 | `.cargo/config.toml` 指定 `C:\Users\xiumi\dev\tools\mingw64\bin\gcc.exe` |
| 依赖源 | `.cargo/config.toml` 用中科大 sparse 镜像 |
| 发布配置 | release：`lto=fat`、`codegen-units=1`、`strip`、**`panic=abort`** |
| 子系统 | `windows_subsystem = "windows"`（无控制台 → **子进程标准句柄无效**，见 §4.3） |
| 单文件主体 | `src/main.rs`（约 2.9 万行，全部 UI）——历史原因，改动时优先查找而非重排；**行号随改动漂移，定位以函数名为准** |

---

## 2. 目录与文件索引（去哪找什么）

### 源码 `src/`
| 文件 | 职责 |
|---|---|
| `main.rs` | 全部 UI（App 状态、更新循环、各页面 `ui_*`、弹窗、托盘、窗口管理）+ 大部分业务编排 |
| `backdrop.rs` | **桌面捕获式材质**（半透明/毛玻璃/亚克力）：worker 线程 BitBlt → 模糊 → 纹理 |
| `process.rs` | 服务器进程启动/停止/stdout 采集（Job Object kill-on-close、日志尾读） |
| `perf.rs` | 进程树 CPU/内存采样 |
| `backup.rs` | 世界/配置备份（**硬链接快照** + 清单/元信息 + 完整性校验 + 保留策略 + 回退；兼容旧版全量/增量 zip） |
| `config.rs` | 配置结构体与默认值（读写实现在 `main.rs::save_config/flush_config/load_config`） |
| `theme.rs` | 调色板、`auto_contrast`、egui `Visuals` 展开 |
| `modrinth.rs` | Modrinth API（搜索/版本/下载地址/**翻译**/**SHA1 指纹**） |
| `download.rs` | 文件下载（单流/分片） |
| `server_download.rs` | 服务端 jar 下载（Vanilla/Paper/Fabric/Spigot…） |
| `serverinfo.rs` | 目录平台识别（Fabric/Forge/NeoForge/Paper、MC 版本、加载器） |
| `crashscan.rs` | 崩溃报告分析（根因归类 + 建议文件定位） |
| `spark_analysis.rs` | Spark 报告解析与可视化 |
| `plugins.rs` | rhai 脚本插件宿主（zip 热加载、毛玻璃示范包） |
| `features.rs` | **测试功能开关注册表**（`BETA_*` 常量 + `REGISTRY`，默认禁用项在这里） |
| `logdb.rs` | SQLite(WAL) 日志库（轮转、查询） |
| `stats.rs` | **仪表盘成就与统计**（本地累计 + 26 项成就判定 + 解锁通知，落盘 `data\stats.json`，可一键关闭） |
| `rcon.rs` | RCON 客户端 |
| `mcmod_db.rs` | 内置 MC 百科离线索引 |

### 其它
| 路径 | 说明 |
|---|---|
| `dist/XMST-0.1.1-alpha.exe` | **交付产物**（唯一该给用户的 exe；0.1.0-alpha 为上一版，仍留在 `dist\` 与 `versions\`） |
| `dist/backup/<时间戳>/` | 上一版 exe 回退副本（保留最近 2 份） |
| `dist/data/` | 运行时数据（config、日志库、`open_diag.log`） |
| `dist/data/plugins/` | **插件目录**（zip 放这里才会被扫描） |
| `data/`（旧位置） | 历史数据目录，别再用（容易"插件/配置消失"的误会） |
| `docs/` | 专题分析文档（材质根因、缺陷盘点、`发版流程.md`、**`UI-审计表.md`（界面信息架构逐项盘点 + 迁移批次）** 等） |
| `vendor/eframe-0.29.1/` | 打过补丁的 eframe（`[patch.crates-io]`，含全局文字描边等改动） |
| `resources/`、`assets/` | 图标等资源 |

---

## 3. 构建 · 部署 · 回退

```powershell
# 1) 构建（必须把临时目录指到工作区内，否则某些 build script 会因权限失败）
New-Item -ItemType Directory -Force F:\XMST\temp\ctmp | Out-Null
$env:TMP="F:\XMST\temp\ctmp"; $env:TEMP="F:\XMST\temp\ctmp"
cd F:\XMST; cargo check --message-format short     # 先 check
cargo build --release                              # 约 3 分钟

# 2) 部署（先备份旧版，保留最近 2 份）
Get-Process XMST* -EA SilentlyContinue | Stop-Process -Force
$bk="F:\XMST\dist\backup\$(Get-Date -Format yyyyMMdd_HHmmss)"; mkdir $bk | Out-Null
Copy-Item F:\XMST\dist\XMST-0.1.1-alpha.exe "$bk\" -Force
Copy-Item F:\XMST\target\release\xmst.exe F:\XMST\dist\XMST-0.1.1-alpha.exe -Force

# 3) 交付前必查
(Get-FileHash F:\XMST\dist\XMST-0.1.1-alpha.exe -Algorithm SHA256).Hash   # 与 target 一致
icacls F:\XMST\dist\XMST-0.1.1-alpha.exe | Select-String 'Mandatory Label' # ★ 必须 Medium
```

- **构建后可删 `target/debug`**（约 1 GB，check 会重建）。
- 交付目录的**完整性标签必须是 Medium**，否则整个程序会被降级运行（§4.1）。
- 改动一律 git 提交，便于按提交回退单文件。
- **发版用 `tools/release.ps1`（详见 `docs/发版流程.md`）**：一条命令完成 构建 → 备份 → 交付（`dist\` + `versions\`）→ 校验 → 提交；默认**不推送**，加 `-Push` 才 `git push`（`-Tag` 同时建并推标签 `v<版本>`）。
- 脚本已把 §7 的校验固化：三处 SHA256 一致、完整性标签必须 Medium（**Low 直接中止**并打印 `icacls … /setintegritylevel M /T /C`）、MOTW 自动 `Unblock-File`、旧产物备份到 `dist\backup\<时间戳>\`（保留 2 份）。
- 用法：`powershell -NoProfile -ExecutionPolicy Bypass -File F:\XMST\tools\release.ps1 [-Version x.y.z-alpha] [-SkipBuild] [-Push] [-Tag]`（可在任意目录调用；本机只有 Windows PowerShell 5.1，装了 pwsh 也可用）。

---

## 4. ★★ 经验与教训（重点：防重蹈覆辙）

### 4.1 低完整性（Low IL）目录 —— 一个原因造成"文件操作 + shell 交互全废"（2026-10-03 结案）

**症状（曾被误判为 4 个独立 bug，白折腾好几轮）**
- 启用/禁用模组、重命名 → `操作失败: 拒绝访问 (os error 5)`，连复制也是 5
- "打开目录/打开所在目录"完全无效；`explorer.exe`、`cmd /c start` **spawn 成功却无窗口**
- `ShellExecuteW` → **5 (SE_ERR_ACCESSDENIED)**；`powershell Invoke-Item` → 退出码 1
- COM `Shell.Application.Explore` → **退出码 0 但窗口不出现**；`Shell.Application.Windows()` 枚举**卡住**
- 每次运行 exe 弹"无法验证发布者"
- **而同一台机器上用 PowerShell 做同样操作全部成功**；读文件、写自己目录（`dist/data`）也一直正常

**根因**：`F:\XMST` 整棵树（含 exe）带 **Low 完整性标签**
```
icacls F:\XMST  →  Mandatory Label\Low Mandatory Level:(OI)(CI)(NW)
```
Windows 中**进程完整性级别取自可执行文件自身的标签** ⇒ 从该 exe 启动的进程是 Low IL：
1. 只能写**同样带 Low 标签**的对象 → 配置/日志写 `dist\data` 居然正常（把排查带偏）；
2. 写普通目录（Medium，如 `D:\Desktop\**`）→ **ACCESS_DENIED(5)**；
3. **UIPI 禁止 Low IL 与 Medium 的 shell 交互** → ShellExecute/COM/explorer 全部"看似成功实则无效"。

**10 秒判定**
```powershell
icacls "<程序目录>" | Select-String 'Mandatory Label'    # 期望 Medium；若是 Low 就是它
# 进程内自检：whoami /groups | findstr "Mandatory Label"  # Low=S-1-16-4096, Medium=S-1-16-8192
```

**修复**（改完**必须重启程序**，运行中的进程保持旧令牌）
```cmd
icacls "F:\XMST" /setintegritylevel M /T /C /Q
```
实测修复效果：写入/改名/复制 OK；`ShellExecute` 5 → **42（>32=成功）**；`Invoke-Item` 1 → **0**。

### 4.2 通用排查法则（本项目用血换来的顺序）
1. **先进程上下文，后代码**：令牌/完整性级别 → 是否在 Job 内及 UI 限制 → 会话/桌面 → ACL / SRP / CFA / 杀软 → **最后**才怀疑 API 用法。
2. **交叉验证**：同一操作**用 PowerShell 做一遍**。PowerShell 成功而程序失败 ⇒ 99% 是**进程上下文差异**，**不是写法问题**——此时抄别的项目（PCL/PCL-CE 等）的实现必然无效（调的是同一个 `ShellExecuteEx`）。
3. **别信"spawn 成功"**：`CreateProcess`/`spawn` 返回 Ok ≠ 目标完成工作。要**验证结果**（本项目用 `FindWindowEx("CabinetWClass")` 数窗口才发现"报成功没窗口"）。
4. **"半可用"是最强误导**：能读不能写、能写自己目录不能写别处 ⇒ 优先怀疑完整性标签/沙箱，而不是代码。
5. **改动前先问"老版本为什么能用"**：能定位到"从哪次改动/哪个路径变更开始坏的"，往往一击命中（本例就是产物搬进 Low 标签目录所致）。
6. **交付/启动自检**：部署脚本检查完整性标签；**程序启动时的 Low IL 检测与提示已实现**（2026-10-04，见 §6.1 第 16 条），且 2026-10-05 起**每台服务器的目录 + 世界目录写权限也在每次启动前自检**（见 §6.1 第 40 条）。

### 4.3 Windows 进程交互的其它坑（已踩）
- **GUI 子系统进程无控制台** → 子进程继承无效标准句柄，可能出现 `0xc0000142`；spawn `explorer/cmd/powershell` 时显式 `.current_dir(安全目录)` + 空标准句柄更稳。
- **子进程工作目录失效**（从已被删除/替换的目录启动）→ 子进程初始化失败，症状与上条类似。
- **`explorer.exe` 不是万能药**：目录打开优先 `ShellExecuteW("open"/"explore")`，失败再退化 `explorer.exe <目录>`；`/select,<文件>` 要用**两个独立参数**。
- **`cmd /C start ""`** 会闪黑框 → 用 `ShellExecuteW` 打开 URL/文件，或给 cmd 加 `CREATE_NO_WINDOW`。

### 4.4 性能：渲染路径里绝不能有重活
egui 的 `ui_*` 每帧执行。曾出现（已修/部分修）：
- 概览页每帧递归遍历 `world/` —— 实测 **5352 文件 / 7.9 GB / 单次 282 ms**（现在 30 s 记忆化）；
- 日志页每帧 `COUNT(*)` + 取 800 行（现在 500 ms 节流缓存）；
- 已接缓存的：`serverinfo::detect_cached`（3 s TTL）、`mod_jar_index`（进 mods 页扫一次后复用）。
- 仍待处理：`dir_stats`（每文件夹 walkdir）。`ui_whitelist_blacklist` 三个折叠标题里的白名单条目数已改读 5 秒 TTL 快照，渲染路径不再读盘（§6.2 第 14 条）。
- **手法**：把结果缓存到 `ServerRuntime`/静态 TTL 缓存，渲染只读缓存；数据变更时主动失效。

### 4.5 数据安全：配置与备份
- `save_config` 必须是**原子写**（tmp → `.bak` → `rename`），**序列化失败绝不写空文件**（曾把配置清空，用户丢过多次）；拖拽滑块要**去抖**（曾每帧整文件重写）。
- `load_config` 解析失败要**先留档坏文件**（`xmst_config.broken_*.json`）再从 `.bak` 恢复，**不要**静默默认值然后覆盖。
- 备份系统（2026-10-04 改造完成）：新格式是 `.mcsrv_backups/snapshots/<yyyyMMdd_HHmmss>/` 的**硬链接快照**——与上一份相比 size + **100ns 精度 mtime** 未变的文件用 `fs::hard_link` 指过去（不占新空间），变化文件才复制；每份带 `meta.json` + `manifest.json`，写完做完整性校验（失败删除该份）；变化判定不再用内容全量哈希（慢）与秒级 mtime（同秒改写漏检）。排除表默认 `logs`/`crash-reports`/`session.lock`/`*.lock`/`cache`/`debug`/`.mcsrv_backups`/`.mcsrv_trash`，既不备份也不参与对比。触发：手动 / **正常关服**（日志 `Stopping server`/`Saving worlds` 或退出码 0，且无新增崩溃报告；有最小间隔防抖）/ 定时；**崩溃默认不备份**（`backup_on_crash`，默认 false）。快照与回退都在后台线程（`spawn_backup`/`spawn_restore` + mpsc），回退前自动生成一份快照、`.mcsrv_trash` 改名失败立即中止、失败文件清单回传界面。旧版全量/增量 zip 仍可列表与链式回退（`build_zip_chain`/`apply_zip`），且不在清理范围内。
  - ★ **硬链接的固有代价**：若源文件被"原地改写"（长度不变、mtime 变），旧快照里指向同一份数据的硬链接也会跟着变。MC 正常保存会重写 region 文件，所以不要把快照目录本身当成防篡改归档；`manifest.json` 的 size 校验只能发现长度变化。
  - ★ 界面每帧不得解析清单：`list_snapshots` 走 `SNAP_TOTAL_CACHE`（按清单大小 + mtime 记忆化），`ui_backup` 另有 2 秒 TTL 列表/总览缓存。
- 备份系统历史坑（已修，勿回退）：快照遍历被"含 backup 的路径"误剪枝 → 增量退化为全量（`has_backup_dir_segment` 只看最后两段 + 精确目录名）。

### 4.6 稳定性
- release 是 **`panic=abort`**：任何 `unwrap()` panic = 进程直接消失。**锁访问一律** `lock().unwrap_or_else(|e| e.into_inner())`（全仓已统一：2026-10-03 的 22 处 + 2026-10-05 收敛的 15 处，见 §6.1 第 26 条）。
- 后台线程请带**上限/池化**（历史上每图标、每服务器各起一个线程）。
- rhai 插件：`max_operations` 必须钳制（已钳 20 万）+ 单帧时间预算，插件分发在 UI 线程。

---

## 5. 诊断入口（不用猜，直接拿数据）

| 入口 | 用法 | 作用 |
|---|---|---|
| `XMST_OPEN_TEST=<目录/文件>` | 设环境变量后启动 exe | 输出 `dist/data/open_diag.log`：**进程完整性、目标目录写入/改名/复制实测、6 种打开方式的返回码** |
| `XMST_CRASHSCAN=<服务器目录>` | 同上 | 只跑崩溃分析并打印结论后退出 |
| `XMST_SHOT=<x.bmp>`（+`XMST_SHOT_EXIT`） | 同上 | 截图到文件后退出（自动化取 UI 快照） |
| `XMST_OPEN_PAGE=<页面> [帧数]` | 设环境变量后启动 exe（页面名：`dashboard`/`servers`/`tunnel`/`logs`/`settings`/`download`/`plugins`/`tools`，另有服务器页签别名 `files`/`console`/`backup`/`players`/`special`） | **无头复现 / 回归**：切到指定页面渲染 N 帧后自动退出；门控未开启或配置里没有服务器时，stderr 会打印「页签会回落到概览」等提示。★ 自动化入口（本变量与 `XMST_SHOT`/`XMST_OPEN_TEST`/`XMST_CRASHSCAN`）**不参与单实例检查**，程序已在运行时也照常跑 |
| `XMST_FLICKER_DIAG=1` | 设环境变量后启动 exe（普通窗口，不切换页面） | **闪屏 / 卡顿诊断**：逐帧写 `<exe 目录>\data\flicker.log`（`frame=` / `theme_diff=` / `ppp=` / `screen=`），并对"整窗重绘类动作"打 `★` 行：`setWindowRgn` / `setPixelsPerPoint` / `setStyle`。★ 行密集 = 就是它们在闪（见 §6.1 第 38 条）；该变量同样跳过单实例检查 |
| `tools\run_with_capture.ps1` | 启动 exe 并把 **stderr** 重定向到文件 | 抓"闪退且无日志"的最后输出（栈溢出/`panic` 前的 stderr，见 §9.2） |
| 应用内 `F12` | 运行中按键 | 截图 |
| `diag/data/bg_debug.log` | 运行时自动写 | 材质：`captures=`、`tex=WxH`、`cap_ms=`、`opacity=`、`accent_ok=`、`ppp=` |
| `data/` 下 `open_diag.log` / 配置 `.bak` / `.broken_*.json` | — | 打开失败诊断 / 配置回退与坏文件留档 |
| `data/stats.json`（+ `.bak` / `.broken_*`） | 运行时自动读写 | **成就与统计**：本地累计计数、在线日期、已解锁成就、总开关 `enabled`（纯本地、可关闭、不联网不上传）；解析失败先留档 `stats.json.broken_<时间戳>` 再从 `.bak` 恢复 |

---

## 6. 已知问题 / 待办（按建议优先级）

### 6.1 已完成（2026-10-04 收尾批 + 2026-10-05 补充；移出待办，仅作对照，勿回退）

1. **备份系统改造**（2026-10-04）：硬链接快照引擎与锁定/转存、关服自动快照、旧版 zip 链式回退（细节见 §4.5）。
2. **托盘隐藏态 tick**（2026-10-04）：`spawn_tray_heartbeat` + `tray_hidden_tick`（1s 节流），隐藏期间自动行为照常推进。
3. **崩溃重启可取消 + 熔断**（2026-10-04）：倒计时窗 + `🛑 取消自动重启`，默认 3 次 / 5 分钟快速熔断。
4. **外部实例识别与接管**（2026-10-04）：扫描 + 三个弹窗（未正常退出 / 似乎已在运行 / 结束外部进程二次确认），接管后纳入停止与崩溃判定。
5. **异常退出不杀服**（2026-10-04，R3）：工具退出不牵连已在运行的服务器（除显式确认）。
6. **一键诊断包**（2026-10-04）：后台打包 + `📂 打开诊断包目录`。
7. **启动前检查**（2026-10-04）：「🩺 启动前检查」窗口，通过/警告/失败/参考计数，不阻断启动。
8. **编辑器编码保护**（2026-10-04）：保存已存在的文本文件按原编码写回 + 先备份（GBK / UTF-8 / BOM，见 §9.5）。
9. **run.bat 模板生成**（2026-10-04）：GBK/CP936 + CRLF + 无 BOM，`MAX_RESTARTS=1`、临时目录指向 `tmp`。
10. **通知不重叠**（2026-10-04）：按实际高度堆叠 + 上限 4 条 + 同类合并。
11. **窄窗口滚动条**（2026-10-04）：内容区右内边距 14pt，滚动条不再被窗口边缘缩放热区抢走。
12. **mods 启用/禁用过滤**（2026-10-04）：`显示: 全部 / 已启用 / 已禁用` + 计数，选择持久化。
13. **仅客户端判定改为只信元数据**（2026-10-04）：`environment` / `displayTest` 为唯一依据，启发式降级为参考线索。
14. **测试功能列表改注册表驱动**（2026-10-04）：由 `features::REGISTRY` 全量生成，补回模组检查更新 / 内网穿透 / 远端备份入口。
15. **检查更新三级匹配**（2026-10-04）：SHA1 指纹优先 → modid 当 slug → 搜索；只报告不替换（原"读 jar 取 modid"的重做方案已由指纹匹配覆盖）。
16. **Low IL 启动自检**（2026-10-04）：启动时判完整性级别，Low 时弹一次提示 + `复制修复命令`（见 §4.1）。
17. **启动方式 auto 回退**（2026-10-04）：bat 失败自动改直连 java（见 §9.6）。
18. **排队优雅停止**（2026-10-04）：启动中点停止改为排队等就绪后优雅停止（`StartPhase` + 排队 / 强制 / 5 分钟超时三条逃生通道，关服快照保持）。
19. `kill_tree` 返回真实结果；日志与 `tail_logs` 的按字节截断统一走 `safe_from`/`safe_to`（中文不再出现半个字）。
20. **主题显式 `is_light`**（2026-10-04）：不再靠正文色亮度反推明暗。
21. **插件缓存目录隔离**（2026-10-04）：`plugins::cache_key` = 可读名 + FNV-1a 64 位后缀，中文名插件不再互相 `remove_dir_all`。
22. **下载文件名消毒**（2026-10-04）：`sanitize_remote_filename()`，`.part` 路径同源（防 zip-slip）。
23. **锁中毒**（2026-10-03）：22 处改 `lock().unwrap_or_else(|e| e.into_inner())`（见 §4.6）。
24. **日志页工具日志化**：**进行中**（另一工作流）。目标见 §9.7；当前页面说明仍写着"含服务器输出汇总"，尚未收口。
25. **`features` 可见性机制删除**（2026-10-05）：`is_visible()` / `FeatureMeta.default_visible` / `FeatureState.visible` 全仓库无消费者，已全部删除——注册表只保留 `default_enabled`，`FeatureState` 只保留 `enabled` / `order`（UI 一律用 `is_enabled`）。
26. **锁访问统一**（2026-10-05）：`main.rs` 的 9 处 `pm.configs.lock()`（4 处 `.lock().ok()` + 5 处 `if let Ok(..) = ..lock()`）、`process.rs::pid`、`modrinth.rs` 翻译缓存 6 处，全部改 `lock().unwrap_or_else(|e| e.into_inner())`；全仓 `\.lock\(\)\.ok\(\)` 归零。
27. **备份全部转存入口**（2026-10-05）：「备份」页「存储与远程」内新增 `☁️ 全部转存到远端`（后台调 `backup::sync_all_snapshots(server_dir, remote_target, None)`，逐份复制、单份失败不中断；完成 toast 报「成功 N 份，复制 X，失败 M 份」，失败明细进工具日志类别「备份」）；远端目标始终只读，不删本地快照。
28. **服务器页签重构**（2026-10-05，审计表 §6 批 2）：`概览 · 控制台 · 文件 · 备份 ·（玩家）· 设置 ·（特殊功能）`；原「服务器状态」与概览内嵌日志合并为「控制台」（性能条 + 日志 + 命令输入 + 折叠的崩溃分析 / TPS 占位）；「自动功能」拆为「备份」与「设置」（自动重启 / 崩溃重启迁入设置）；删除「🧬 属性」占位子页签与 `ui_players_props_disabled`。
29. **界面信息架构重组批 3 / 批 4**（2026-10-05）：批 3 统一工具条（6 个一级页对齐 `page_toolbar`）+ 设置分组规范化（基础组展开、高级/实验组折叠，一页最多 5 组）；批 4 弹窗与文案收敛（标题层级、按钮顺序"取消在左/危险在右"、危险色与二次确认统一，`清空日志` / `清空日志库` 补确认，取消隧道 `Shift` 直删后门已堵）。**已知取舍**：服务器「设置」页的其余分区（服务器属性 / 启动脚本 / Java 设置等）仍为**标题 + 分隔线**风格，未改成折叠组；`server.properties` 的 27 个字段已按 网络 / 规则 / 世界 / RCON 拆成 4 个折叠组。
30. **仪表盘成就 / 里程碑系统**（2026-10-05，审计表 §8）：新模块 `stats.rs` 落盘 `data\stats.json`（原子写 tmp → `.bak` → rename，解析失败先留档 `stats.json.broken_<时间戳>` 再回 `.bak`）；26 项成就（服务器数 1/3/5/10/20、启动 10/100/1000、时长 10/100/1000 小时、下载 1/10/50/100、模组 1/10/100、首次快照 / 首次回退 / 首次穿透连通 / 首次诊断包 / 首次崩溃定位、连续 7 / 30 天在线、关服自动快照 100 次）；事件发生时累加并判定新解锁（右下角工具内通知 + 工具日志类别 `成就`），渲染只读 `stats::snapshot()`；仪表盘统计下方新增「🏆 成就」卡片（已解锁 X / Y + 每行 3 个网格 + 未解锁灰显 + hover 条件与进度 + 查看全部折叠 + 「启用成就统计」开关，关闭只停记录不清数据）。**公告位为占位**：`fn announcement_area` 空实现（优先本地 `data\announcement.md`，联网拉取未决定，未写任何联网代码）。
31. **界面密度收敛**（2026-10-05，"区域都变大/变厚"修复）：批 3 的统一工具条把控件硬编码成 26pt，同时把全局行距抬到 6pt、按钮上下内边距抬到 3pt，三项叠加让所有页面明显变厚。现在：`ui_ctl_h` 默认 **26 → 18**（= 该设置引入前的控件高度）、`item_spacing.y` 系数 0.75 → 0.375（默认 8 → 3pt）、`button_padding.y` 系数 0.5 → 0.25（默认回到 1pt）；工具条不再硬编码高度，改为跟随全局控件高度（`ui_ctl_h()` 读 `UI_CTL_H_PT`，由每帧界面令牌写回；**不要**在 `ui.add_sized([w, …])` 的实参位置写 `ui.spacing()`，会与 `&mut ui` 借用冲突 E0502）。旧配置仍是"间距 8 + 控件高 26"这组旧默认值的会一次性收敛到 18（`migrate_ui_density_defaults` + `ui_density_tight` 标记，手动调过的不动）。
32. **自动化入口不参与单实例检查**（2026-10-05）：`XMST_OPEN_PAGE` / `XMST_SHOT` / `XMST_OPEN_TEST` / `XMST_CRASHSCAN` 任一存在时跳过 `single_instance_check`。此前程序已在运行（多数情况停在托盘）时，诊断实例会"唤醒已有窗口并静默 `exit(0)`"，表现为 **exit=0 但一帧没渲染、没有 stderr、没有截图** —— 会把无头回归误判成通过（本批第一次截图/回归就踩到了）。
33. **备份页三处"点不动/跳错"**（2026-10-05，用户报"快照无法删除 / 锁定后无法解锁 / 打开目录跑到文档"）：
    - ★ **`bg_busy` 键不一致**（真 bug）：`spawn_pin_snapshot` / `spawn_sync_snapshot` 用**快照全路径**入队（`pin:<全路径>`），回传处理却用**目录名**删除（`pin:<名字>`）→ 键永久残留，`contains_key` 早退 → 同一份快照的**第二次点击（解锁/再次转存）被静默吞掉**（无 toast、无日志、无反应）。现统一为 `pin:{idx}:{名字}` / `sync:{idx}:{名字}`。**改这类"入队-回传"键时，两处必须逐字一致。**
    - ★ **混合分隔符**：`SNAPSHOTS_DIR` 原为 `.mcsrv_backups/snapshots`（正斜杠），于是快照路径全是 `D:\...\.mcsrv_backups/snapshots\<时间戳>`。文件 API 能接受，**`explorer.exe` 不能** —— 解析失败后它退化成打开「文档」（`open_diag.log` 实录：目标 exists=true、`explorer.exe spawn OK`、`COM Explore Some(0)`，用户看到的却是文档）。现常量统一反斜杠，`abs_path` 再做一次分隔符归一（兜用户手填的 `/`）。
    - ★ **按钮被挤出可视区**：备份列表每行"长详情 + 5 个按钮"排在同一 `ui.horizontal`，超过 1100px 内容宽度（`page_center_1100`）后按钮画在可视区外 → 表现就是"删除/解锁点了没反应"。现拆两行：信息行用 `horizontal_wrapped`，按钮独占一行。手动删除补了工具日志（成功/失败各一条），便于下次定位。
    - 另有 `backup.rs` 新增 2 个回归测试（`ui_flow_probe`：`delete_backup` 接列表给出的路径、`pin→is_pinned→unpin` 闭环）——两条后端链路实测都是好的，所以上述"删不掉/解不开"根因在 UI 层。
34. **界面重排 + 设置改造**（2026-10-05 第二批，用户 10 条清单）：概览拆成「快捷方式 + 服务器信息 + 折叠的排障组」（启停移出）；控制台把 `启动/停止/强制结束` 置顶、「服务器状态（CPU/内存采样）」改折叠分组、删 TPS/MSPT 占位；成就卡片与通知从界面移除、成就系统挂到 `features::BETA_ACHIEVEMENTS`（默认关，打开也只累计不展示）；服务器设置把 `run.bat`/启动方式/JVM 参数/`user_jvm_args.txt` 归入「🚀 启动参数」，run.bat 缺失时才给生成/导入，`user_jvm_args.txt` 仅在存在时显示，**改 JVM 参数会同步写进 run.bat**（`jvm_synced`）；Java 选择＝run.bat 里的路径优先 → 按 MC 版本自动匹配（1.20.5–1.21.11→21 / 1.17–1.20.4→17 / ≤1.16.5→8 / 26.1+→25）→ 全局兜底，选定后**覆盖写回 run.bat 的 `JAVA_PATH`**；设置-界面把 圆角/间距/高度/字号 合成「UI」组（拖动只预览、点「应用」才生效），英文选项禁用，动画速度改 6 档预设（**预设 1 = 旧的默认 2.0**，滑块非线性）；工具内通知统一 右侧滑入→停留→右侧滑出；取消"顶栏搜索"（设置搜索移进左侧分区导航、日志过滤贴着列表、下载页重复搜索框删除）；删除解释工具内部机制的补充文案。
35. **"XMST 拉起 run.bat 必失败"取证结论**（2026-10-05，`dist\data\launch.log` 实录 + 复现实验）：
    - 事实链：`cmd /c run.bat` **确实跑起来了**（stdout 首行是 bat 自己的 `====` 横幅，说明 `where java` 检查已通过）→ ~3 秒后 cmd 自己 `退出码=1`、**stderr 空**、`logs\latest.log` 大小/mtime **无变化**（说明 java 从未走到日志初始化）→ auto 模式下的回退没生效，因为当时解析出的直连 java 不存在（`<服务器目录>\java\bin\java.exe`）。
    - 交叉验证：同一台机器上 `where java`（`E:\Games\Minecraft\Library\JDK\OpenJDK21`）、`java -version`、以及模板那行 `call "java" %JVM_ARGS% -version`（含 `"-Djava.io.tmpdir=…"` 引号写法）**在带控制台时全部成功**（含 `-Xms1G -Xmx15G -XX:+UseG1GC`）。
    - 因此剩余差异只剩**工具的启动形态**：`CREATE_NO_WINDOW` + stdin/stdout/stderr 全管道。java 静默退出、无 stderr、无 latest.log，指向"无控制台句柄下 java/MC 侧提前结束"，而不是 bat 写法或 PATH 问题。
    - **下一步（已内置工具）**：用「🧪 启动诊断」一次跑 4 组（① 生产组合 ② 去掉 Job ③ 去掉 CREATE_NO_WINDOW ④ 绕过 cmd/bat 直接 java），结果写 `data\launch.log`；哪一组能出 `Done` 就是根因所在。
36. **★ run.bat 必失败的最终根因（2026-10-05 已修，取代第 35 条的推测）**：`process.rs` 里读子进程输出的线程用 `read_line` 读 stdout/stderr，**遇到 GBK/非 UTF-8 字节就返回 Err 并结束线程** → 读端被关闭 → 子进程管道破裂 → `cmd`（及 bat 里的 `call "java"`）以**退出码 1** 提前结束，且 stderr 空、`logs\latest.log` 大小/mtime 无变化 —— 与第 35 条取证到的现象完全吻合（bat 的 `echo` 中文横幅就是第一处非 UTF-8 字节）。修法：4 处读取线程统一改 `read_line_lossy`（`read_until` + `from_utf8_lossy`），并把 stdout/stderr 前 5 + 末 10 行与标准句柄组合写进 `launch.log`（提交 `dc799bb`）。**教训：GUI 进程读子进程输出必须按字节读 + 容错解码，不能因为"某行不是 UTF-8"就终止读取线程。**
37. **界面细节修复**（2026-10-05 第三批）：
    - ① 「设置 → 界面」的平滑动画速度改回 **1–3 线性滑块（默认 2 = 原默认手感）**，删掉"预设 1–6"档位与非线性映射；越界旧值（>3 / <1）自动收敛进区间。
    - ② 修 **UI 预览显示不正确**：预览字号曾把倍率乘了两次（`scaled_font(14.0 * k, 14.0)`，k 已是倍率）→ 预览字比真实界面大一圈；圆角曾固定按 `12 × 幅度` 画，而真实控件是 `4 × 幅度`、容器 `6 × 窗口幅度`。现按"待应用 ÷ 当前生效"的比例缩放字号，圆角与 `theme::apply` 同源，示例按钮/输入框也用待应用圆角绘制。
    - ③ 修 **「设置 → Java」整页空白**：`match self.settings_side` 里出现**重复的 `SettingsSide::Java` 分支**，第一个空分支让真正的 `java_jvm` / `java_list` 分组成为不可达代码（编译期只有 warning，界面上一片空白）。**合并/大改之后务必用 `cargo check` 的 `unreachable pattern` 警告自查一遍。**
    - ④ 成就统计的**启动初始化与退出落盘**也挂到 `BETA_ACHIEVEMENTS`（关闭时连 `stats.json` 都不创建、不写「成就」日志）。
    - ⑤ 再清一批机制解释文案：`已锁定为独立归档（不再随源文件变化）…`、`已转存快照 X（本次复制 Y）`、`正在后台清理超出保留策略的快照…`、`快照已开始在后台生成（完成后提示）`、`关闭后过渡立即完成，后台刷新率降到最低`、Java/JVM 与 Java 列表两个分组的机制 hint、`服务器控制台日志在 服务器 → 控制台；穿透日志在…`、`（暂存 Java 条目）`。
38. **★ 界面闪屏（2026-10-05 第三批，用 `XMST_FLICKER_DIAG` 逐帧诊断定位）**：诊断入口见 §5。实测（2560×1440 / 系统缩放 125%）：
    - ★ **启动时 DPI 拉扯**：`App::new` 里 `base_ppp = ctx.pixels_per_point()` 拿到的是 egui 当时的默认值，而 **eframe 会在启动头几帧陆续送来系统原生缩放** → 程序把 125% 显示器强行按 100% 渲染：`pixels_per_point` 1.25 → 1.0、客户区 2048×1152 → 2560×1440，`SetWindowRgn` + 字体/布局整体重算各来一次 ⇒ **肉眼就是"启动闪一下"**（顺带界面比系统缩放小 20%）。修法：基线改成**系统原生缩放**（`viewport().native_pixels_per_point`），且**连续 3 帧不变**后才应用字号缩放（`ppp_stable_frames`）。修后：**0 次 setPixelsPerPoint**。
    - ★ **`SetWindowRgn` 每帧重设**：该 API 会让整窗重绘一次；拖动缩放/最大化时 `w/h` 每帧都变 ⇒ 缩放全过程持续闪。修法：几何变化后**稳定 250ms** 再一次性重设（`geom_changed_at` + `request_repaint_after(260ms)` 保证空闲时也能补上圆角）。修后：启动到空闲**只有 1 次 SetWindowRgn**（改前 2 次）。
    - `ctx.set_style` 改为只在「控件间距 / 控件高度」真的变化时调用（原先每帧克隆整个 `Style` 并替换 `Arc<Style>`）。
    - **诊断法**：`$env:XMST_FLICKER_DIAG=1` 启动 → `data\flicker.log` 逐帧记录 `frame=/theme_diff=/ppp=/screen=`，并对三类"整窗重绘类动作"打 `★` 行；`★` 行密集 = 就是它们在闪。
39. **Java 选择按服务端版本自动取 + run.bat 双向同步（2026-10-05 第三批）**：
    - MC 版本在「服务器 → 设置 → Java 设置」里**从服务端目录自动识别**（`serverinfo::detect_cached`，只在 `mc_version` 为空时自动写入，识别不出才回落输入框），旁边「重新识别」按钮用 1ms TTL 强制重跑一次。
    - 「Java 来源」下拉：按服务器 MC 版本匹配的条目标注 `✓ 匹配`，自动模式下直接显示"自动（按 MC 版本用 Java 21）→ <会选中的条目>"，用户不用猜。
    - 全局 Java 列表的「版本」从手填改成 **Java 8 / 17 / 21 / 25 下拉** + 「识别」按钮（`java_major_of_path()` 跑一次 `java -version`），名称留空时按版本自动命名。
    - **run.bat → 工具** 反向同步：保存 run.bat 时用 `extract_bat_jvm_args()` 把 `set "JVM_ARGS=…"`（兼容 `JAVA_OPTS`）回填到本服务器「JVM 参数」。★ **引号规则必须与 `replace_bat_jvm_args` 同源**：模板的值自带引号（`"-Djava.io.tmpdir=%~dp0tmp"`），只剥 `set` 自己的那一个收尾引号。回归测试 `bat_jvm_args_extract` 覆盖"写出→读回"往返。
40. **★★ 数据安全：关服回档专项（2026-10-05 第四批，源自 ATM10 诊断报告）** —— 一次真实事故，四条独立问题叠加；下面的改法即约定，**勿回退**：
    - **停服超时不再静默强杀**：`process.rs::stop_gracefully_wait` 超时返回 `StopOutcome::StillRunning`，**绝不 taskkill**；`main.rs` 弹「服务器仍在退出中」→「继续等待（再等一轮，可反复）/ 强制结束（才 `kill_tree` + 警告级日志）」。超时值＝每服务器配置 `stop_timeout_secs`（默认 **300**，区间 30..=1800）。旧函数 `stop_gracefully()` 保留"超时即强杀"语义，**只允许**用在"强杀可接受"的路径（回退前停服、工具退出静默关服）；用户手动点「停止服务器」必须走 `stop_gracefully_wait` + 确认。历史上这里硬编码 30 秒 + 静默 `taskkill /PID x /T /F`，ATM10（3.3 GB region / 20+ 维度，`sync-chunk-writes=true`）一次完整 `stop` 存档要 1 分钟以上 ⇒ 存档截断 ⇒ **下次启动回档**。
    - **关服快照必须等进程完全退出**：`wait_server_fully_exited(dir, 60s)` 确认「该目录没有 java 进程（`java_processes_for_dir`，命令行匹配）+ `world\session.lock` 最近 3 秒没被刷新」，超时**跳过**本次快照并记警告；等待期间就 `exit_snapshot_inflight.insert(idx)` —— 否则退出流程会在等待线程还在跑时判定"全部停止完毕"提前退出，把最后一份快照打断。
    - **启动前必须查残留**：`request_start` → `pre_start_issue`（java 进程扫描 + `session.lock` 新鲜度）命中就弹「仍要启动 / 取消」（**默认取消、取消在左**），绝不静默启动。四条路径都要走：手动启动、`--autostart`、崩溃重启倒计时结束、run.bat 失败回退。两个实例抢同一个 `session.lock`/region 就是"谁最后写算谁的"。
    - **`use_private_tmp` 默认改为关闭**：带「文件夹保护 / 勒索防护」的机器会拒绝写 `<服务器目录>\tmp`（JNA 建临时 dll → `UnsatisfiedLinkError`，服务器起不来）。它当初要绕的根因已修（§6.1 第 36 条）。**旧配置里显式 `true` 的一律保留**（只加一行橙色提示）。
    - **写权限自检**：`probe_world_writable` 是**只读探测** —— 在 `world\` 建/删自己的临时文件 + 用 `OpenOptions::write(true)` **只打开**一个既有 `.mca`（不写内容、不改任何既有文件）。「文件夹保护」拦的正是"打开既有存档准备写入"，只探新建文件会漏判。
    - **不完整快照**（缺 `manifest.json`，工具写一半退出留下的，如报告的 `20261005_145301`）在列表里标 `[不完整]`、**回退按钮置灰**；工具条「🧹 清理不完整」二次确认后只删这些目录（`backup::delete_backup` 本身会拒绝 `snapshots` 之外的路径，删除前再复核 manifest 仍缺失）。
    - 教训：**任何"等不到就强杀"的自动兜底，只要动的是服务器进程，都必须改成"告诉用户 + 让用户选"**；`kill_tree` 只应由用户显式确认，或工具退出（窗口已最小化、无法交互，此时程序代为强杀并记警告日志）触发。

### 6.2 仍待办（按建议优先级）

1. **界面信息架构重组**：逐项盘点、合并结论与迁移批次见 `docs\UI-审计表.md`（一级导航 7 → 6、服务器页签收敛、统一工具条、设置分组折叠）。**进度（2026-10-05）**：批 1（一级导航 7 → 6，「插件 / 日志」收进「工具」二级）、批 2（服务器页签重构：`概览 / 控制台 / 文件 / 备份 /（玩家）/ 设置 /（特殊功能）`）、**批 3（统一工具条 + 设置分组规范化）与批 4（弹窗与文案收敛）已完成**（见 §6.1 第 28、29 条）；**已知取舍**：服务器「设置」页的其余分区（服务器属性 / 启动脚本 / Java 设置等）仍为标题 + 分隔线风格，未改成折叠组；`server.properties` 的 27 个字段已按 网络 / 规则 / 世界 / RCON 拆成 4 个折叠组。两处死代码 `ui_perf`（含 `ServerTab::Perf`）与 `ui_players_props` 已删除，`ui_players_props_disabled` 占位（`🧬 属性` 子页签）也已随批 2 删除。
1a. **仪表盘增强** → **成就已落地（2026-10-05，`src\stats.rs` + 仪表盘「🏆 成就」卡片，见 §6.1 第 30 条）；公告位已预留但未实现**（`fn announcement_area` 空实现，优先本地公告文件 `data\announcement.md`，联网拉取尚未拍板，未写任何联网代码）；详见 `docs\UI-审计表.md` §8。
1b. **仪表盘与内网穿透保留为一级导航**（已在 UI 重组决定中确认，重组时不要动这两项）。
2. **发布 0.1.1**：产物已在 `dist\XMST-0.1.1-alpha.exe` 与 `versions\0.1.1-alpha\`，等确认后走 `tools/release.ps1`。
3. **服务器目录完整性自检** → **已落地（2026-10-05，见 §6.1 第 40 条）**：`check_server_integrity_before_start` 在每次启动前对**该服务器目录 + 自带 Java 运行时目录**做 `server_integrity_lows` 检查（Low 时弹提示 + 复制 `icacls … /setintegritylevel M` 命令），并额外做**世界目录写权限只读探测**（`probe_world_writable`）。残留可做项：把这两项也接进「🩺 启动前检查」的自动预检（当前写权限已在预检第 9 项，完整性级别在预检第 8 项）。
4. **`XMST_OPEN_PAGE` 退出加固**：收尾走 `ViewportCommand::Close`，被"有服务器运行"的二次确认拦住后靠 300 帧兜底 `std::process::exit(0)`；该兜底跳过配置去抖与日志落盘（现在只在进入退出前 `flush_config()` 一次）。
5. **远端备份覆盖面**：`BETA_REMOTE_BACKUP` 目前只对旧版 zip 生效；快照是目录，未做远端同步。
6. **硬链接快照的固有代价**：源文件"原地改写"会让旧快照同步变化（见 §4.5）；要做防篡改归档需改为复制或加写时校验。
7. **`features` 可见性机制** → **已删（2026-10-05）**：`is_visible()` / `FeatureMeta.default_visible` / `FeatureState.visible` 全仓库无消费者，已全部删除，注册表与 `FeatureState` 只保留"启用"语义（`enabled` / `order`）。详见 §6.1 第 25 条。
8. **锁访问规约** → **已统一（2026-10-05）**：`main.rs` 的 9 处 `pm.configs.lock()`（4 处 `.lock().ok()` + 5 处 `if let Ok(..) = ..lock()`）、`process.rs::pid`、`modrinth.rs` 翻译缓存的 6 处全部收敛为 `lock().unwrap_or_else(|e| e.into_inner())`；全仓 `\.lock\(\)\.ok\(\)` 为 0。详见 §6.1 第 26 条。
9. **`DownloadState` 解包过密**：`self.dl` 上 `.as_mut().unwrap()` 49 处 + `.as_ref().unwrap()` 57 处（合计 106；此前记的"96 处"只是其中一部分）。`panic=abort` 下任一处失手即闪退，应改为一次 `let Some(dl) = ...` 或集中取引用。
10. **`windows_version_text` 缓冲契约** → **已注明（2026-10-05）**：`main.rs` 该函数的文档注释已写明"缓冲区是**函数内局部数组**、每次调用重新分配、**不做跨帧/跨调用复用**；`RegGetValueW` 最后一个参数是**字节数**（初值 `buf.len()*2`，成功后被写回实际字节数）；任一读取失败返回空串、不 panic"。改动前按该契约执行。
11. **`GetDiskFreeSpaceExW` 指针用法** → **已修（2026-10-05）**：`backup.rs` 改为三个出参各传真实 `ULARGE_INTEGER` 变量的地址（`*mut _`），返回值按 `QuadPart()` 解引用取 64 位，不再有"64 位值被当 32 位读"的隐患。
12. **主线程栈大小** → **结论：不可安全改动（2026-10-05 复核）**：`main` 直接 `eframe::run_native`，Win32 窗口过程、托盘消息线程、DWM 与桌面捕获都建立在"UI 在主线程"这一前提上；把整个入口搬进 `std::thread::Builder::stack_size(..)` 会同时牵动窗口与 GL 上下文的归属，回归风险大于收益，故**保持现状**，并把"在递归点加深度上限"作为首选手段（§9.2 的栈溢出即由 `log_rows_cached` 自我递归引起）。**可选方案（未采用）**：在 `.cargo/config.toml` 的链接参数里给主线程加栈——Windows GNU 目标加 `-Wl,--stack=<字节数>`，只动构建配置、不改代码结构。
13. **rhai 插件递归未实测**：`max_operations` 已钳 20 万 + 单帧预算，但脚本内深度递归的实际表现未测。
14. **剩余每帧读盘** → **白名单部分已修（2026-10-05）**：`ui_whitelist_blacklist` 三个折叠标题的条目数改读 5 秒 TTL 快照（`wl_lists` / `WlListsCache`，增删名单后 `invalidate_wl_lists` 立即失效），渲染路径不再读盘。**仍待处理**：`dir_stats`（每文件夹 walkdir）。
15. **下载健壮性剩余项**：下载 client 的总超时不应管大文件；落盘应统一走 `.part` → 校验字节/哈希 → `rename`（文件名消毒已完成，见 §6.1 第 22 条）。

---

## 7. 交付前检查清单

- [ ] `cargo check` 无 error，`cargo build --release` 成功
- [ ] `dist/XMST-0.1.1-alpha.exe` 与 `target/release/xmst.exe`、`versions/0.1.1-alpha/` 下同名文件 **三处 SHA256 一致**（见 §9.3）
- [ ] 旧版已备份到 `dist/backup/<时间戳>/`（保留 2 份）
- [ ] **exe 与目录完整性标签 = Medium**（`icacls … | Select-String 'Mandatory Label'`）
- [ ] 无 Zone.Identifier（MOTW）；若对外分发建议用普通 zip（并提示用 7-Zip 解压）
- [ ] 已 git 提交
- [ ] 插件/配置目录正确：`dist/data/plugins`、`dist/data/`

---

## 8. 省 token 工作法（协作约定）

> 目标：同样工作量花更少输入 token。历史上曾出现"单会话从早拖到深夜"，越到后面每轮越贵（长历史被反复重读）。

### 8.1 会话
- **一个任务一个会话**：做完一件事开新会话；状态靠 `AGENTS.md`（约定/经验）+ git（改动）+ `CHANGELOG.md`（成果）传递，不要靠长对话记忆。
- **先计划后执行**：先给 3-5 步计划，确认后再动手，避免"试探—纠正—再试探"。
- **结论落文件**：新坑与新约定随手写进本文件；下个会话读文件远比重读历史便宜。

### 8.2 输入
- **截图贵**（约 1-2k tokens/张，且每轮重读）：只贴关键几行；UI 问题用"页面 + 位置 + 期望"描述。
- **大日志/大文件不要整份发**：先说明需要哪几行（给 `Select-String` 的结果）。
- **命令输出必须截断**：`Select-Object -First/-Tail N`、`cargo --message-format short`。

### 8.3 执行
- **不要给同一个文件并行开多个代理**（会互相覆盖）✗：串行，或一人一个文件范围；写入前重新读取并复核交集。
- **攒批再构建**：`cargo build --release` 约 4 分钟，小改动先攒着一起构建。
- **先拿证据再改**：优先取原始输出（stderr、日志、诊断），不要先猜后验。
- **子代理的长报告写文件**（`temp\*_report.md`），只把结论与关键行带回主线。

### 8.4 换成自有 API 后
- **命中 prompt cache**：稳定内容放会话前部（本文件、项目约定），不要频繁改动前缀。
- **分层用模型**：检索/样板用便宜或本地模型，判断与审查交给贵的模型。
- **流程脚本化**：`tools/release.ps1`（发版）、`XMST_OPEN_PAGE`（无头回归）这类"一条命令完成"的入口越多越省。

---

## 9. 2026-10-04 事故与教训补记

### 9.1 低完整性（Low IL）第二次：`D:\Desktop`
`D:\Desktop` 整棵树带 Low 标签 → 其中 `java.exe` 以 Low IL 运行 → **连 `logs\latest.log` 都写不出来**，表现为"工具启动服务器连日志都没有、退出码 1、stderr 为空"，而同一条 `run.bat` 在 PowerShell 里能正常启动。修复：

```powershell
icacls "D:\Desktop" /setintegritylevel M /T /C /Q
```

判定：`icacls <目录> | Select-String 'Mandatory Label'`。**凡"同一操作在别的 shell 能成、在程序里不成"，先查目录/文件的完整性标签**（见 §4.1、§4.2）。

### 9.2 栈溢出不走 panic 钩子
`log_rows_cached` 一度自我递归 → 主线程栈溢出 → 进程直接消失（`0xC00000FD`、`thread 'main' has overflowed its stack`），而 `data\crash.log` **没有任何记录** ✗。排查"闪退且无日志"时：用 `tools/run_with_capture.ps1` 重定向捕获 stderr，或用 `XMST_OPEN_PAGE=<页面> [帧数]` 无头复现。

### 9.3 部署要核对三处 SHA 并防占用
程序仍运行时 `Copy-Item` 覆盖 `dist\*.exe` 会**失败**（曾出现 `dist` 旧、`versions` 新）。流程：结束进程 → 带重试复制 → 核对 `target\release\xmst.exe`、`dist\...exe`、`versions\<版本>\...exe` **三处 SHA256 一致**。

### 9.4 `cargo test` 用系统默认 TEMP
把 `TMP/TEMP` 指到 `F:\XMST\temp\ctmp`（构建需要）会让 `encoding_selfcheck` 的若干测试因 rename 失败而红。**构建**用工作区临时目录，**测试**用系统默认 TEMP。

### 9.5 文本编码
`safe_from`/`safe_to` 是 UTF-8 边界安全切片工具，所有按字节截断处都必须使用。工具生成的 `run.bat` 必须写 **GBK/CP936 + CRLF + 无 BOM**；保存任何已存在的文本文件都要**按原编码写回 + 先备份**（曾有用户的 `run.bat` 被按 UTF-8 重写后中文乱码、cmd 解析失败、服务器完全起不来）。

### 9.6 已知问题：工具用 `cmd /c run.bat` 启动会失败
同一台机器上 `cmd /c run.bat` 在 PowerShell 里能启动服务器，但由本工具（无控制台 GUI + 管道句柄）拉起时 `cmd` 约 0.7s 退出、java 从未启动。已提供：每个服务器的**启动方式**（`auto` 默认，bat 失败自动改直连 java / `java` / `bat`）、`data\launch.log` 启动诊断与「🧪 启动诊断」按钮。根因（Job/句柄层差异）未完全定位，回退方案可用。

### 9.7 日志页 = 工具日志
左侧「日志」应展示**工具自身运行事件**（启动/停止、备份快照、崩溃熔断、配置保存、下载、隧道、插件、更新检查…）；服务器控制台日志属于「服务器 → 日志」，穿透日志属于隧道页。三者不要混排。

### 9.8 不要用 PowerShell 文本管道改源码（会把中文与行尾一起毁掉）
`(Get-Content 文件 -Raw).Replace(...) | Set-Content 文件 -NoNewline -Encoding utf8` 在 **Windows PowerShell 5.1** 下会把 UTF-8 源码按系统 ANSI（中文机器上是 GBK）解码再按 UTF-8 写回：**中文全部变成乱码**、文件头多出 BOM、部分 CRLF 被吃掉成 LF，且这种转换**不可逆**（解码期的替换字符已经丢失原字节），只能逐字重写。

- 改文件（尤其 `.rs`）一律用编辑工具：按字节读写、保留原行尾，不碰编码。
- 确需脚本批量替换时，读写都用显式 UTF-8 无 BOM：
  `$c = [System.IO.File]::ReadAllText($p, [System.Text.UTF8Encoding]::new($false))` → `[System.IO.File]::WriteAllText($p, $c, [System.Text.UTF8Encoding]::new($false))`（本仓 `src\main.rs` 是 **CRLF**，替换串里换行要写 `` `r`n ``）。
- 判据：改完立刻 `icacls` 之外再看三件事 —— 文件无 BOM、`CRLF` 计数与改前一致、`Select-String` 能搜到某个已知中文串。
- 注意 DSH 的编辑工具是**按行尾归一化后的字面匹配**：某次 `edit` 报成功，说明当时的文件里确实存在 `old_string`；反过来说，用脚本重放历史 `edit` 时若匹配不上，往往是漏掉了同一会话里用 shell 改文件的那几步（本仓一次事故就源于漏着重放 3 次 `WriteAllText`）。