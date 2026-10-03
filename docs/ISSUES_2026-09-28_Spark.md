# Spark 相关问题记录（2026-09-28）

> 状态：已记录，待交接文档传递完毕后统一修复。
> 来源：反馈 + 截图 + 代码审阅（main.rs / spark_analysis.rs / features.rs）。

## 问题 1：Spark 输出文件点击分析报错「解析文件失败 — 系统找不到指定路径 (os error 3)」

- **状态**：✅ 已修复（2026-09-28）。点击文件时以 `sc.dir.join(full)` 补全绝对路径再解析；`cargo check` 通过。
- **现象**：特殊功能页点击 .sparkprofile / .sparkhealth 文件后，底部红色报错，无法分析。
- **根因（已定位）**：
  - `scan_spark_files`（main.rs:6518）扫描 `config\spark` / `plugins\spark` 时，列表里存的是**相对子目录** `sub`（即 `"config\\spark"` / `"plugins\\spark"`），并未拼上服务器绝对目录 `sc.dir`。
  - UI 点击处（main.rs:6425）`let full = format!("{}\\{}", dir, name);` 拼出的是相对路径，`parse_spark_file` 里 `std::fs::read(path)` 以进程当前工作目录为基准解析，找不到文件 → `os error 3`。
- **修复方向**：扫描时保存完整绝对路径（`sc.dir.join(sub)`），或点击时用 `sc.dir.join(&full)` 再解析；`spark_analysis.path` 展示时再显示相对名。

## 问题 2：Spark 性能分析 UI 排版重构

- **状态**：✅ 已修复（2026-09-28）。ui_special 重构：总折叠「Spark 性能分析」+ 子折叠（安装检测/性能分析/输出文件/分析结果预览）；左侧 300px 分析控制（时长 DragValue 5..=3600 + 开始分析 + 进度条 + 强制停止 + 完成提示），右侧输出文件列表 + 解析预览；到期自动发送 stop --save-to-file 并延迟刷新文件列表；`cargo check` 通过。
- **需求**：
  1. 上方为功能总折叠（特殊功能页整体可折叠）。
  2. 特殊功能区内所有特殊功能均支持独立折叠，避免页面过大。
  3. 功能区标题显示为「Spark性能分析」。
  4. 展开后布局（左右分栏）：
     - **左侧顶部**：分析按钮（点击后向服务器控制台发送 `/spark profiler start` 开始分析）→ 右侧为**分析时长**输入（用户可设）→ 再右侧为**分析进度条**（按分析时长百分比显示）→ 另有**强行停止**按钮（提前发送 `/spark profiler stop`）。
     - **下方左侧**：spark 输出文件列表（.sparkprofile / .sparkhealth）。
     - **文件列表右侧**：分析按钮（点击后解析该文件，结果显示在右侧预览区）。
     - **右侧**：预览区，对解析内容做合适排版展示。
- **涉及实现**：控制台命令发送（`/spark profiler start` / `stop`）、分析会话定时器与进度条、UI 左右分栏重构、折叠（总折叠 + 子折叠，可复用 setting_section 折叠动画思路）。
- **注意**：开始分析前应确认服务器在运行（命令通过服务器控制台输入）。

## 问题 3：Spark 与特殊功能移出「测试功能」，转为正式功能

- **状态**：✅ 已修复（2026-09-28）。features.rs BETA_SPECIAL/FEATURE_SPARK 注册项 group Beta→Server、default_enabled 改 true；设置页测试功能列表移除 BETA_SPECIAL 项；特殊功能页未启用时点击「启用 Spark 分析」直接启用、不再弹 beta 确认窗；`cargo check` 通过。
- **现状**：`BETA_SPECIAL`（beta.server.special 总开关）与 `FEATURE_SPARK`（server.special.spark 子开关）注册在 features.rs 测试功能体系，UI 中归入「特殊功能」测试页，启用需弹 beta 确认。
- **需求**：特殊功能页正式化，不再属于测试功能（去掉测试确认弹窗、改为默认启用/可配置，文案去除「测试」字样）。
- **涉及**：features.rs 开关注册与默认值、main.rs ui_special 相关确认弹窗逻辑、设置页「测试功能」分组展示。

## 附：本次已读取代码位置备忘

- `main.rs:6518` scan_spark_files；`main.rs:6553` parse_spark_file；`main.rs:6339` ui_special；`main.rs:6425` 文件点击拼路径。
- `spark_analysis.rs:112` parse_spark_file（fs::read 报错源头）。
- `features.rs:98-99` BETA_SPECIAL / FEATURE_SPARK 定义；`features.rs:393/401` 注册。
*（内容由AI生成，仅供参考）*
