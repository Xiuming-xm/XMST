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
## 最近变更记录
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
- **代码结构**：根目录 F:\XMST，src/ 共 13 个模块——main.rs（应用主逻辑/UI/运行时管理，已超万行）、config.rs、modrinth.rs（Modrinth API/翻译/分页）、rcon.rs、plugins.rs（rhai 插件）、logdb.rs（SQLite 日志）、backup.rs、perf.rs、theme.rs、process.rs（子进程/日志尾随）、download.rs、server_download.rs、features.rs（功能开关注册表）；dist/data/xmst_config.json 为核心配置，dist/ 产出 xmst.exe 与「修暝的服务器工具.exe」两份同名 exe。
- **文档**：HANDOVER.md（本文档，跨会话交接总纲）、会话交接_2026-09-26.md（阶段 0-7 记录）、README.md、docs/TODO-frp平台API接入.md。

## 2. 交付基线（务必以此为准）
| 项目 | 说明 |
| dist 两份 exe | `F:\XMST\dist\xmst.exe` 与 `F:\XMST\dist\修暝的服务器工具.exe`（同名同内容，覆盖须逐份核对，SHA256 一致） |
| 最近构建时间 | 2026-09-27 18:33（阶段12），14,675,968 字节 |
| 阶段11 基线 | 2026-09-27 11:55（2,006,912 字节，SHA256 933DC229E65F15A1B1A6232D9D7ED7D331E1D144C94717FD5B8F6B0C55511ECB） |
| 阶段8 基线 | 2026-09-27 12:46（1,968,512 字节，SHA256 8CF3B78D3078EFA8A48836277B0EAD17AC00D7B655576776A9834FF7914F7343） |
| 阶段C 基线 | 2026-09-26 21:51（1,912,192 字节，SHA256 C2BD6C45…） |
| 会话交接基线 | 2026-09-26 13:34:21（1,951,232 字节，SHA256 60380853E274312ADC38E10BA7F0FB66A01865F399F7C5974AC5B39ECA2A54C5） |
| 图标 | `assets\xmst.ico`（黑底白 e，多尺寸 16..256，embed-resource 嵌入）；`assets\icon_32.rgba` / `icon_64.rgba` 为同源运行时图标（include_bytes! 读取） |
| 工具链要求 | Rust stable（实测 1.98.1）+ x86_64-pc-windows-gnu + MinGW（cargo/config.toml 指定 linker） |
## 3. 已完成功能清单（按入口/模块）
| 构建产物 | `target/release/xmst.exe`，部署名两份 exe |
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
## 附录. 重建补全说明（2026-09-27）
- 本版在保留原 HANDOVER.md 全部未损坏内容（最近变更记录阶段 12/11/8/C/图标、章节 7/8、试错速查 1-9）基础上，补全：截断原因分析、章节 1（项目概述）、章节 2（交付基线）、章节 3（已完成功能清单）、章节 4（版本条件热点表）、章节 5（已知问题与待复测）、阶段 0-10 变更记录、试错速查 10-16。
- 来源：会话交接_2026-09-26.md（阶段 0-7）、README.md（2026-09-20/21）、docs\TODO-frp平台API接入.md、HANDOVER.md 残留尾部与历次会话记录。
- 阶段 9/10 无任何记录佐证，如实标注缺失，未臆造；「2026-09-25 自动备份禁用完全隐藏」条目完整内容亦缺失，待人工补充。
*（内容由AI生成，仅供参考）*