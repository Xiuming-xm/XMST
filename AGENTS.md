# XMST 项目指南（索引 · 经验 · 排查手册）

> 面向接手本项目的 AI / 开发者。**只保留"去哪找什么"和"踩过的坑"**，不再记录逐轮改动流水账（历史见 git log）。
> 最近一次大修：2026-10-03（低完整性目录事件 + 审计修复，见 §4、§5）。

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
| 单文件主体 | `src/main.rs`（约 2 万行，全部 UI）——历史原因，改动时优先查找而非重排 |

---

## 2. 目录与文件索引（去哪找什么）

### 源码 `src/`
| 文件 | 职责 |
|---|---|
| `main.rs` | 全部 UI（App 状态、更新循环、各页面 `ui_*`、弹窗、托盘、窗口管理）+ 大部分业务编排 |
| `backdrop.rs` | **桌面捕获式材质**（半透明/毛玻璃/亚克力）：worker 线程 BitBlt → 模糊 → 纹理 |
| `process.rs` | 服务器进程启动/停止/stdout 采集（Job Object kill-on-close、日志尾读） |
| `perf.rs` | 进程树 CPU/内存采样 |
| `backup.rs` | 世界/配置备份（快照 + 增量镜像 + zip + 回退） |
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
| `rcon.rs` | RCON 客户端 |
| `mcmod_db.rs` | 内置 MC 百科离线索引 |

### 其它
| 路径 | 说明 |
|---|---|
| `dist/XMST-0.1.0-alpha.exe` | **交付产物**（唯一该给用户的 exe） |
| `dist/backup/<时间戳>/` | 上一版 exe 回退副本（保留最近 2 份） |
| `dist/data/` | 运行时数据（config、日志库、`open_diag.log`） |
| `dist/data/plugins/` | **插件目录**（zip 放这里才会被扫描） |
| `data/`（旧位置） | 历史数据目录，别再用（容易"插件/配置消失"的误会） |
| `docs/` | 专题分析文档（材质根因、缺陷盘点等） |
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
Copy-Item F:\XMST\dist\XMST-0.1.0-alpha.exe "$bk\" -Force
Copy-Item F:\XMST\target\release\xmst.exe F:\XMST\dist\XMST-0.1.0-alpha.exe -Force

# 3) 交付前必查
(Get-FileHash F:\XMST\dist\XMST-0.1.0-alpha.exe -Algorithm SHA256).Hash   # 与 target 一致
icacls F:\XMST\dist\XMST-0.1.0-alpha.exe | Select-String 'Mandatory Label' # ★ 必须 Medium
```

- **构建后可删 `target/debug`**（约 1 GB，check 会重建）。
- 交付目录的**完整性标签必须是 Medium**，否则整个程序会被降级运行（§4.1）。
- 改动一律 git 提交，便于按提交回退单文件。

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
6. **交付/启动自检**：部署脚本检查完整性标签；程序应在启动时检测 Low IL 并提示（**待办 §6**）。

### 4.3 Windows 进程交互的其它坑（已踩）
- **GUI 子系统进程无控制台** → 子进程继承无效标准句柄，可能出现 `0xc0000142`；spawn `explorer/cmd/powershell` 时显式 `.current_dir(安全目录)` + 空标准句柄更稳。
- **子进程工作目录失效**（从已被删除/替换的目录启动）→ 子进程初始化失败，症状与上条类似。
- **`explorer.exe` 不是万能药**：目录打开优先 `ShellExecuteW("open"/"explore")`，失败再退化 `explorer.exe <目录>`；`/select,<文件>` 要用**两个独立参数**。
- **`cmd /C start ""`** 会闪黑框 → 用 `ShellExecuteW` 打开 URL/文件，或给 cmd 加 `CREATE_NO_WINDOW`。

### 4.4 性能：渲染路径里绝不能有重活
egui 的 `ui_*` 每帧执行。曾出现（已修/部分修）：
- 概览页每帧递归遍历 `world/` —— 实测 **5352 文件 / 7.9 GB / 单次 282 ms**（现在 30 s 记忆化）；
- 日志页每帧 `COUNT(*)` + 取 800 行（现在 500 ms 节流缓存）；
- 仍待处理：`serverinfo::detect`、`dir_stats`（每文件夹 walkdir）、白名单 JSON 每帧读盘、**逐个打开所有 mods/*.jar**（§6）。
- **手法**：把结果缓存到 `ServerRuntime`/静态 TTL 缓存，渲染只读缓存；数据变更时主动失效。

### 4.5 数据安全：配置与备份
- `save_config` 必须是**原子写**（tmp → `.bak` → `rename`），**序列化失败绝不写空文件**（曾把配置清空，用户丢过多次）；拖拽滑块要**去抖**（曾每帧整文件重写）。
- `load_config` 解析失败要**先留档坏文件**（`xmst_config.broken_*.json`）再从 `.bak` 恢复，**不要**静默默认值然后覆盖。
- 备份系统已知问题（**待修，用户会补需求**）：快照遍历被"含 backup 的路径"误剪枝 → 增量退化为全量、检测不到删除；`.mcsrv_trash` 的 `rename` 未检查结果；回退在 UI 线程执行。

### 4.6 稳定性
- release 是 **`panic=abort`**：任何 `unwrap()` panic = 进程直接消失。**锁访问一律** `lock().unwrap_or_else(|e| e.into_inner())`（已完成 22 处）。
- 后台线程请带**上限/池化**（历史上每图标、每服务器各起一个线程）。
- rhai 插件：`max_operations` 必须钳制（已钳 20 万）+ 单帧时间预算，插件分发在 UI 线程。

---

## 5. 诊断入口（不用猜，直接拿数据）

| 入口 | 用法 | 作用 |
|---|---|---|
| `XMST_OPEN_TEST=<目录/文件>` | 设环境变量后启动 exe | 输出 `dist/data/open_diag.log`：**进程完整性、目标目录写入/改名/复制实测、6 种打开方式的返回码** |
| `XMST_CRASHSCAN=<服务器目录>` | 同上 | 只跑崩溃分析并打印结论后退出 |
| `XMST_SHOT=<x.bmp>`（+`XMST_SHOT_EXIT`） | 同上 | 截图到文件后退出（自动化取 UI 快照） |
| 应用内 `F12` | 运行中按键 | 截图 |
| `diag/data/bg_debug.log` | 运行时自动写 | 材质：`captures=`、`tex=WxH`、`cap_ms=`、`opacity=`、`accent_ok=`、`ppp=` |
| `data/` 下 `open_diag.log` / 配置 `.bak` / `.broken_*.json` | — | 打开失败诊断 / 配置回退与坏文件留档 |

---

## 6. 已知问题 / 待办（按建议优先级）

1. **备份三连**（等用户补需求）：增量剪枝误伤自身、`.mcsrv_trash` rename 未校验即覆盖、回退阻塞 UI 30 s+、`apply_zip` 整文件读入内存。
2. **模组"检查更新"**：现已在 `设置 → 测试功能 → 模组检查更新`（`beta.server.mod_update`，**默认禁用**）。重做方案：读 jar 内 `fabric.mod.json`/`mods.toml` 取 **modid** → 查 Modrinth 项目/版本（按 MC 版本+加载器过滤）→ 与本地版本比对；交互改为**只检查并在弹窗里报告可更新版本**（用户选择，不自动替换）。
3. **启动自检 Low IL**：检测到低完整性时直接提示修复命令（避免再次误判）。
4. **插件缓存 key**：`plugins.rs::cache_key` 把所有非 ASCII 字符替换成 `_` → 两个中文名插件共用目录且互相 `remove_dir_all`，需追加名称哈希。
5. **剩余每帧重活**：`serverinfo::detect`、`dir_stats`、`load_list`(白名单 JSON)、`scan_mod_jar_index`（逐个开 jar）。
6. 主题：明暗判定目前取自正文色亮度（`theme.rs`），自定义配色下可能反相，应改为显式 `is_light` 字段。
7. `kill_tree` 无条件返回 true（谎报成功）；`tail_logs` 可能切断多字节字符（中文日志出现 �）。
8. 下载健壮性：下载 client 的总超时不应管大文件；应下到 `.part` 校验字节/哈希后 `rename`；远端文件名需消毒（防 zip-slip）。

---

## 7. 交付前检查清单

- [ ] `cargo check` 无 error，`cargo build --release` 成功
- [ ] `dist/XMST-0.1.0-alpha.exe` 与 `target/release/xmst.exe` **SHA256 一致**
- [ ] 旧版已备份到 `dist/backup/<时间戳>/`（保留 2 份）
- [ ] **exe 与目录完整性标签 = Medium**（`icacls … | Select-String 'Mandatory Label'`）
- [ ] 无 Zone.Identifier（MOTW）；若对外分发建议用普通 zip（并提示用 7-Zip 解压）
- [ ] 已 git 提交
- [ ] 插件/配置目录正确：`dist/data/plugins`、`dist/data/`
