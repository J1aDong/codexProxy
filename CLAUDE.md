# OpenWolf

@.wolf/OPENWOLF.md

This project uses OpenWolf for context management. Read and follow .wolf/OPENWOLF.md every session. Check .wolf/cerebrum.md before generating code. Check .wolf/anatomy.md before reading files.


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
  - `unified.rs`：**通用中间抽象层**，负责把 Anthropic 输入拍平成统一请求结构（text/tool/image/thinking 等）
  - `providers/`：各上游协议适配层
    - `CodexAdapter`：`UnifiedChatRequest -> Responses API`
    - `OpenAIChatAdapter`：`UnifiedChatRequest -> Chat Completions API`
    - `GeminiAdapter`：`UnifiedChatRequest -> Gemini API`
    - `AnthropicAdapter`：`UnifiedChatRequest -> Anthropic Messages`
  - `codex/response.rs`：Codex SSE/JSON -> Anthropic SSE/JSON
  - `openai.rs`：OpenAI Chat SSE/JSON -> Anthropic SSE/JSON
  - `gemini.rs`：Gemini SSE/JSON -> Anthropic SSE/JSON
  - `anthropic.rs`：Anthropic 透传后端与 identity response transformer
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
4. 先把 Anthropic 请求转换为 `transform/unified.rs` 中的 `UnifiedChatRequest`
5. 再由 `transform/providers/` 中对应 adapter 编码成目标上游协议
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
- Tauri 版本是主线实现，`backup/` 为历史参考
- `main/src/transform/unified.rs` 只允许承载**协议无关**抽象，不允许混入任何单一上游的私有策略
- 任何 provider 专属能力必须放在各自 adapter / backend 钩子里，不能反向污染 unified 层

---

## 6. 快速启动与定位入口
- 开发启动：`cd fronted-tauri && npm run tauri dev`
- 核心服务启动：`cd main && cargo run --bin codex-proxy-server`
- 关键排查入口：
  - 路由/上游行为：`main/src/server.rs`
  - LB 行为：`main/src/load_balancer/mod.rs`
  - 通用抽象：`main/src/transform/unified.rs`
  - 上游请求编码：`main/src/transform/providers/mod.rs`
  - Codex 响应转换：`main/src/transform/codex/response.rs`
  - 桌面配置与热更新：`fronted-tauri/src-tauri/src/proxy.rs`

---

## 7. 一句话版本（给未来的我）
这是一个“以 Anthropic 入口兼容 Claude Code、以 Rust 代理层通过 `UnifiedChatRequest` 汇聚多模型上游并支持可控 failback”的本地网关工程，Tauri 负责可视化配置，`main` 负责协议与路由真逻辑。

---

## 8. 流式优化开关策略（2026-02 补充）
- 面向普通用户：**不在前端 UI 暴露流式优化细开关**，避免增加理解成本与误操作风险。
- 默认策略：流式优化能力在后端按默认值启用（如事件帧解析、心跳、日志采样、指标等）。
- 兼容原则：如客户端明确要求 JSON 语义，应保持非流式返回，不做破坏性覆盖。
- 运维原则：仅在排障或回滚场景下，通过配置文件/运行时配置手动覆盖这些开关，不作为日常操作项。

---

## 9. 黏连治理工程方法（2026-02 补充）
- 当前路线不是“补丁地狱”，而是标准工程路径：**Hardening + Invariant Enforcement + Observability**。
- 如果后续再出现黏连，不代表无解；优先按以下闭环处理：
  1. 先用 `[TransformDiag]` 定位是哪条护栏没兜住；
  2. 把现场样本固化为 replay test；
  3. 再做机制级收敛（状态机/通道隔离/事件边界），而不是只加一条字符串规则。
- 目标是把“新增样本 -> 可复现 -> 可回归 -> 可观测”变成固定工程流程。

---

## 10. `unified.rs` 抽象原则（重要）
- `main/src/transform/unified.rs` 是 **协议无关的统一输入模型层**，它的职责只有：
  - 归一化消息角色
  - 归一化文本、图片、tool call、tool result、thinking
  - 为各上游 adapter 提供统一、稳定、可测试的输入结构
- `unified.rs` **不允许**承载以下内容：
  - Codex 专属字段，例如 `prompt_cache_key`、`store`、`previous_response_id`、`service_tier`
  - OpenAI Chat / Gemini / Anthropic 任一协议的私有请求字段
  - 某个上游独有的缓存策略、路由 hint、session 粘连策略
  - 仅为了单一 provider 才存在的 prompt 注入或兼容补丁
- 如果某个能力只对单一协议成立：
  - 放到 `transform/providers/` 里的对应 adapter
  - 或放到该 provider 的 backend / response transformer 钩子中
  - 不要回写到 `UnifiedChatRequest`
- 当前约定：
  - 图片路径标准化、base64/data URI 归一化属于通用输入归一化，可以留在 `unified.rs`
  - Codex 的 prompt cache / stateful chaining / session hint 属于 Codex 专属能力，必须只在 Codex adapter 路径中处理
