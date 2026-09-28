---
AIGC:
    Label: "1"
    ContentProducer: 001191440300708461136T1XGW3
    ProduceID: f60842f40542d18996ccf15309f5e3de_9f1107afb4cc11f193fb525400393706
    ReservedCode1: TxelLsXEHz+wq8ya7Eu9slMQlL+jzYgPlOZ2X5SN6Qdn20AgE93+ilKaAsYWuR1oZTJhQk4YNp2/A46lnRn2X8vWSqIIICRfRgraEkdcoozBcg/j/fHxla3YEW0KiaERN7J3/A0WT3C9OcL18TxmGpE0cX2vIV1bMRO/i00e1GmqPMZgM52oSIX1akU=
    ContentPropagator: 001191440300708461136T1XGW3
    PropagateID: f60842f40542d18996ccf15309f5e3de_9f1107afb4cc11f193fb525400393706
    ReservedCode2: TxelLsXEHz+wq8ya7Eu9slMQlL+jzYgPlOZ2X5SN6Qdn20AgE93+ilKaAsYWuR1oZTJhQk4YNp2/A46lnRn2X8vWSqIIICRfRgraEkdcoozBcg/j/fHxla3YEW0KiaERN7J3/A0WT3C9OcL18TxmGpE0cX2vIV1bMRO/i00e1GmqPMZgM52oSIX1akU=
---

# XMST（修暝的服务器工具）

Minecraft 服务器运行管理工具（Rust + egui，单文件 exe），原名 MCServerManager，现名 XMST。

## 功能模块

- 服务器管理：添加/启动/停止服务器、JVM 参数、控制台输入与日志查看
- 备份：全量 + 增量备份，支持限速与低优先级执行、备份列表（日期/类型）、删除（确认）、加载回退（确认）
- 穿透：内网穿透隧道管理
- 设置：JVM 默认参数、日志行数、语言等

## 项目结构

- `src/main.rs`（~2385 行）— 应用主逻辑：UI（页签/窗口）、服务器运行时管理、日志渲染、设置
- `src/process.rs` — 子进程管理（隐藏窗口启动、stdout/stderr 捕获、stdin 命令）、日志文件尾随读取器 `LogFileTail`
- `src/backup.rs` — 备份引擎（全量 + 增量镜像对比、节流复制、后台线程、低优先级）
- `src/perf.rs` — 服务器进程性能采样（独立性能页图表）
- `src/config.rs` — 全局配置（服务器列表、JVM 参数、日志行数上限等，持久化为 JSON）

## 日志链路（2026-09-21 改造）

背景：通过 `cmd /c run.bat -> java` 启动服务器时，Java 侧 stdout 在管道场景下可能被缓冲，日志在启动早期之后停滞；关服时未 flush 的缓冲直接丢失，UI 日志卡死（服务器实际可进可关）。诊断证据：`logs/latest.log` 文件正常写满完整关服过程（589 行），确认根因是 stdout 管道输送问题，与行数裁剪/渲染/贴底无关。

改造：服务器日志源从 stdout 管道改为**文件尾随** `logs/latest.log`（log4j FileAppender 实时写盘）。

- 新增 `process::LogFileTail`：按字节偏移增量读取，自动处理文件轮转（`len < offset` 时偏移归零）、行尾未闭合拼接（`pending`）、`\r` 进度条清理、单帧 1MB 上限、pending 64KB 强制切行保护。
- `main.rs` 每帧优先 tail：`tail_logs` 返回 `Some(n)` 且 `seen_data` 为真（文件通道曾读到数据）时**丢弃 stdout 管道数据**（内容重复且格式不一致）；文件通道未生效（启动初期文件未建、非 MC 服务器）时 **stdout 管道兜底**；连续 60 帧（曾生效后 20 帧）打不开文件判定通道无效，返回 `None` 永久回退 stdout。
- `start_server` 成功时重建 `rt.log_tail`；关服后 `log_tail` 保留，关服日志仍可继续读到 UI（修复前这部分被 stdout 缓冲吞掉）。

已知行为：文件通道首次激活时会把文件已有内容全部读入，与 stdout 已显示的启动早期行有短暂重复（约十几行），属可接受的切换开销。

代码位置：

- `process.rs`：`LogFileTail::new / tail_logs / seen_data`、`drain_logs_discard`
- `main.rs`：`ServerRuntime.log_tail` 字段（~129）、`start_server` 初始化（~396）、`App::update` 每帧拉取（~1030）

## 构建

```
cargo build --release
```

产物：`target/release/xmst.exe`，部署名 `XMST.exe`（旧部署名 `修暝的服务器工具.exe`）。
工具链要求：Rust stable + x86_64-pc-windows-gnu，MinGW 链接器。

## 已知问题

- 构建警告（既有，非 2026-09-21 引入）：`process.rs:136` unused_mut（child 锁变量）、`main.rs:143` `backup_working` 未读、`main.rs:212` `new_server_dir` 未读、`backup.rs:38` `files` 未读、`backup.rs:657` `dir_size` 未用、`process.rs:168` `drain_to_string` 未用。建议方向：后续任务顺手清理或加 `#[allow(dead_code)]` 标注。
- 日志文件通道首次激活时与 stdout 已显示行有短暂重复（见"日志链路"章节），可接受；如要彻底消除，需改为启动即禁用 stdout 显示，代价是丢失 run.bat 的 echo 输出。
- 本次改造尚未做真机运行验证（需用户在新会话启动服务器确认日志持续滚动、关服日志完整显示）。

## 最近变更记录

### 2026-09-21 日志读取修复（stdout → tail latest.log）

1. 根治日志卡死：新增 `process::LogFileTail` 文件尾随读取器，服务器日志源从 stdout 管道改为 `logs/latest.log` 文件增量读取（绕开 Java stdout 缓冲），stdout 管道保留作兜底（启动初期 / 非 MC 服务器 / 文件通道失效时）。
2. 每帧拉取逻辑改造：文件通道生效后丢弃 stdout 数据避免重复；关服后文件通道继续工作，关服日志不再丢失。
3. 涉及文件：`src/process.rs`（新增 `LogFileTail`、`drain_logs_discard`，清理 unused import `Sender`）、`src/main.rs`（`ServerRuntime.log_tail` 字段、`start_server` 初始化、`App::update` 拉取循环）。
4. 构建验证：`cargo build --release` 通过（EXIT=0），产物 `target/release/xiuming-server-tool.exe`（14.26MB）。
5. 遗留问题：未做真机运行验证；既有 warning 见"已知问题"。

### 2026-09-20 备份策略与改名

1. 备份策略改为「首次全量 + 后续增量」：
   - 增量以镜像清单对比文件变化，仅打包变化文件；检测到全量基线丢失时，下一次备份自动补全量。
   - 备份存于服务器目录 `.mcsrv_backups\`，镜像与清单存于 `.mcsrv_backups\snapshot`、`manifest.json`。
2. 移除备份大小限制功能（`max_size_mb`），旧配置加载时自动忽略该字段。
3. 降低备份性能消耗：
   - 备份移入后台线程，UI 不再阻塞；
   - 新增限速设置 `throttle_mbps`（默认 30 MB/s），复制按节流拷贝；
   - 备份线程设置为低优先级（`THREAD_PRIORITY_BELOW_NORMAL`），降低与服务器进程争抢资源导致的爆内存风险。
4. 工具更名「修暝的服务器工具」：
   - 窗口标题、顶部栏、单实例互斥体名（`Local\XiumingServerTool_SingleInstance`）、提示文案、exe 文件名均已更新；
   - 编译产物 `xiuming-server-tool.exe`，部署名 `修暝的服务器工具.exe`。

### 待处理

- 旧测试备份（约 35GB，14 个 zip）位于 `D:\Desktop\[7.3] AllTheMod10\.mcsrv_backups`，超出回收站容量无法由工具移入回收站，需在主界面或资源管理器中手动删除。
*（内容由AI生成，仅供参考）*
