# Codex Proxy 项目初始化说明（/init）

## 1. 我做这个项目的初衷
这个项目的核心目标是：**让 Claude Code 以 Anthropic Messages 协议入口，稳定接入多个不同上游（Codex / Gemini / Anthropic 透传）**，并在一个本地代理里统一完成：

- 协议转换（请求与流式响应）
- 模型映射（opus/sonnet/haiku）
- 多端点负载均衡与故障切换（failback）
- 桌面可视化配置（Tauri）

一句话：**不改 Claude Code 使用习惯，只改本地代理层，就能灵活切换和汇聚不同模型供应通道。**

---

## 2. 当前项目架构（现状）

### 2.1 总体分层
- `fronted-tauri/`：桌面前端（Vue）
- `fronted-tauri/src-tauri/`：Tauri 命令层（启动/停止代理、热更新配置、导入导出配置）
- `main/`：Rust 核心代理库与服务（真实协议转换与路由逻辑）
- `backup/`：历史 Node.js 版本（归档参考）

### 2.2 核心模块（`main/src`）
- `server.rs`：入口 HTTP 服务、请求生命周期、路由选择、上游调用、SSE 回传
- `transform/`：
  - `codex.rs`：Anthropic -> Codex Responses 转换、Codex SSE -> Anthropic SSE
  - `gemini.rs`：Anthropic -> Gemini 转换、Gemini SSE -> Anthropic SSE
  - `anthropic.rs`：Anthropic 透传后端
  - `processor.rs`：消息块转换（text/tool/image/thinking 等）
- `load_balancer/mod.rs`：按 slot（opus/sonnet/haiku）进行多候选端点选择与健康状态管理
- `models/`：请求体与内容块类型定义（含 tool_use/tool_result/thinking/image 等）
- `logger.rs`：请求/响应日志与调试日志能力

### 2.3 运行拓扑
`Claude Code -> http://localhost:8889/v1/messages -> Codex Proxy -> (Codex/Gemini/Anthropic Upstream)`

---

## 3. 关键请求流程（当前实现）
1. 接收 `/v1/messages` 或 `/v1/messages/count_tokens`
2. 识别输入模型归属 slot（opus/sonnet/haiku）
3. 若启用 LB：在当前 slot 候选中选择端点（不跨 slot）
4. 按 converter + operation 统一解析上游 URL
5. 执行协议转换（含 tools、image、reasoning、model 映射）
6. 调用上游，流式响应再转换回 Anthropic SSE
7. 根据返回状态更新 LB 路由健康（冷却/恢复/短退避）
8. 在可重试错误下执行**同请求内 failback**（同 slot 内切换下一个候选）

---

## 4. 实际已完成能力（截至当前代码）

### 4.1 协议与转换能力
- Anthropic Messages 输入适配
- Codex Responses 上游适配（主路径）
- Gemini 上游适配
- Anthropic 透传适配
- 工具调用链路（tool_use / tool_result -> function_call / function_call_output）
- 图片输入块转换
- 流式 SSE 转换回 Anthropic 事件格式

### 4.2 模型与推理映射
- 按 `opus/sonnet/haiku` 做模型族识别
- Codex 模型映射（默认：`gpt-5.3-codex` / `gpt-5.2-codex` / `gpt-5.1-codex-mini`）
- Anthropic 模型映射（可留空透传）
- Gemini 模型预设与映射
- reasoning_effort 映射与覆盖

### 4.3 负载均衡与故障切换
- 三个 slot 独立候选池：`opus` / `sonnet` / `haiku`
- 端点策略：`max_concurrency`、`error_threshold`、`cooldown_seconds`、`transient_backoff_seconds`
- 路由状态：Healthy / Constrained / Cooldown
- 识别模型不可用、鉴权问题、配额问题等不可用信号
- **同请求内 failback（同 slot 内）**
- `404 Route not found` 归类为路由不可用并快速切换

### 4.4 URL 规范化（已统一）
- 按 converter + operation 统一解析上游 URL：
  - codex message -> `/responses`
  - codex count_tokens -> `/responses/input_tokens`
  - anthropic message -> `/messages` 或 `/v1/messages`
  - anthropic count_tokens -> `/messages/count_tokens`
  - gemini message/countTokens 路径规则

### 4.5 稳定性与运维能力
- `count_tokens` 上游失败时可回退估算（可配置开关）
- probe 请求本地忽略（可配置）
- 本地模型冷却表（429 等）
- 配置热更新（无需重启进程，端口不变时生效）
- 运行日志、路由日志、请求摘要日志
- 配置导入/导出与桌面 UI 管理

### 4.6 当前已修复的重要兼容点
- Codex 输入中的 `thinking` 块在出口规范化为上游支持的 `summary_text`（避免 `Invalid value: 'thinking'`）

---

## 5. 当前边界与设计约束
- 负载切换默认**不跨 slot**（opus 不自动降到 sonnet/haiku）
- `codex-request.json` 中模板指令依赖仍是核心约束
- Tauri 版本是主线实现，`backup/` 为历史参考

---

## 6. 快速启动与定位入口
- 开发启动：`cd fronted-tauri && npm run tauri dev`
- 核心服务启动：`cd main && cargo run --bin codex-proxy-server`
- 关键排查入口：
  - 路由/上游行为：`main/src/server.rs`
  - LB 行为：`main/src/load_balancer/mod.rs`
  - Codex 转换：`main/src/transform/codex.rs`
  - 桌面配置与热更新：`fronted-tauri/src-tauri/src/proxy.rs`

---

## 7. 一句话版本（给未来的我）
这是一个“以 Anthropic 入口兼容 Claude Code、以 Rust 代理层汇聚多模型上游并支持可控 failback”的本地网关工程，Tauri 负责可视化配置，`main` 负责协议与路由真逻辑。
