# 待办：frp 平台 API 接入（先准备，不正式做）

> 状态：仅调研准备阶段，未进入实现。
> 来源：XMST 任务清单 3.3（Q7 用户决策：先准备不正式做）。

## 目标背景

XMST 内网穿透页已支持 frp 集中管理（单 frpc 进程 + frpc.toml 多隧道）。
后续若要做「frp 平台 API 接入」，目的：
- 从 frp 服务端（frps）侧获取隧道在线状态、连接数、流量统计、客户端列表；
- 在 XMST「状态」页或穿透页展示服务端视角数据；
- 未来可支持多服务器/多 frps 实例管理。

## 调研准备清单（未执行）

- [ ] 收集 frp 官方 frps 面板 API 文档（fatedier/frp GitHub：dashboard 端口、auth、/api 路由）
- [ ] 对比常见 frp 面板方案（frp-panel、frps-onekey、frp 官方 dashboard）的 API 形态
- [ ] 确认第三方 frp 服务商（如花生壳类平台）是否开放隧道管理 API
- [ ] 评估 XMST 接入形态：仅展示 vs 增删改隧道；鉴权方式（token / basic auth）
- [ ] 确认配置项设计：frps_dashboard_addr / frps_api_token 等字段是否加入 GlobalConfig

## 注意事项

- 不引入非必要运行时依赖，优先走内置 HTTP 客户端（reqwest 已可用则复用）。
- API Token 属敏感字段，配置文件中按现有 token 字段同等处理。
- 涉及平台 API 变动风险，实现前再次核实文档。
*（内容由AI生成，仅供参考）*
