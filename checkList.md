# Transformer 功能覆盖检查清单（仅基于官方 Anthropic / Claude Code 文档）

> 用途：用于检查“Anthropic Messages / Claude Code -> 你的转换器 -> 上游”这一层是否覆盖了官方协议与官方 CLI 能力面。
> 说明：**本清单不参考你当前仓库实现**，只按官方文档拆检查项；因此它适合作为“目标覆盖面”与回归检查模板，而不是现状判定。
> 建议状态：`[ ] 未测` / `[x] 通过` / `[!] 有差异` / `[-] 不适用`

---

## 0. 官方文档范围

### Anthropic API
- Messages / Tool use / Extended thinking / Streaming / Stop reasons / Vision / PDF / Files API / Token counting / MCP connector

### Claude Code
- Common workflows / Plan Mode / Images in CLI / MCP / Subagents / Agent Teams

### 官方来源
- Messages / Tool use overview: https://platform.claude.com/docs/en/agents-and-tools/tool-use/overview
- Implement tool use: https://platform.claude.com/docs/zh-CN/agents-and-tools/tool-use/implement-tool-use
- Extended thinking: https://platform.claude.com/docs/en/build-with-claude/extended-thinking
- Streaming: https://platform.claude.com/docs/en/build-with-claude/streaming
- Stop reasons: https://platform.claude.com/docs/en/build-with-claude/handling-stop-reasons
- Vision: https://platform.claude.com/docs/zh-CN/build-with-claude/vision
- PDF support: https://platform.claude.com/docs/zh-CN/build-with-claude/pdf-support
- Files API: https://platform.claude.com/docs/en/build-with-claude/files
- Citations: https://platform.claude.com/docs/en/build-with-claude/citations
- Token counting: https://platform.claude.com/docs/en/build-with-claude/token-counting
- MCP connector (Messages API): https://platform.claude.com/docs/zh-CN/agents-and-tools/mcp-connector
- Claude Code common workflows: https://code.claude.com/docs/en/tutorials
- Claude Code MCP: https://code.claude.com/docs/zh-CN/mcp
- Claude Code subagents: https://code.claude.com/docs/zh-CN/sub-agents
- Claude Code agent teams: https://code.claude.com/docs/en/agent-teams

---

## 1. Messages API 基线能力

### 1.1 基础请求形态
- [ ] 支持 `POST /v1/messages`
- [ ] 支持基础字段：`model` / `max_tokens` / `messages`
- [ ] 支持 `system`
- [ ] 支持 `stream: true`
- [ ] 支持 `stop_sequences`
- [ ] 支持多轮 `user` / `assistant` 对话历史
- [ ] 支持 `messages[].content` 两种形态：纯字符串、内容块数组

### 1.2 内容块基础类型
- [ ] `text`
- [ ] `image`
- [ ] `document`
- [ ] `tool_use`
- [ ] `tool_result`
- [ ] `thinking`
- [ ] `redacted_thinking`（如果你要覆盖扩展思考相关兼容）

### 1.3 响应基本字段
- [ ] 正确处理 `message.id`
- [ ] 正确处理 `role=assistant`
- [ ] 正确处理 `content[]`
- [ ] 正确处理 `stop_reason`
- [ ] 正确处理 `stop_sequence`
- [ ] 正确处理 `usage`

检查备注：
- Anthropic 官方把工具、图片、文档、thinking 都视为 `content` block，而不是额外 role。

---

## 2. Client Tools / Server Tools 覆盖

### 2.1 工具定义结构
- [ ] `tools[]` 顶层参数支持
- [ ] client tool 定义支持：`name` / `description` / `input_schema`
- [ ] 可选 `input_examples`
- [ ] `name` 是否满足官方正则：`^[a-zA-Z0-9_-]{1,64}$`
- [ ] 可选 `strict: true` 场景是否支持

### 2.2 tool_choice 行为
- [ ] `tool_choice: {"type":"auto"}`
- [ ] `tool_choice: {"type":"any"}`
- [ ] `tool_choice: {"type":"tool","name":"..."}`
- [ ] `tool_choice: {"type":"none"}`
- [ ] `disable_parallel_tool_use=true` 行为正确

### 2.3 Client tool 调用闭环
- [ ] assistant 返回 `tool_use` block 时能被正确识别
- [ ] 响应 `stop_reason="tool_use"` 时进入工具执行分支
- [ ] 工具执行后可回送 `tool_result`
- [ ] `tool_result.tool_use_id` 正确对应到原 `tool_use.id`
- [ ] `tool_result.is_error=true` 错误场景正确处理

### 2.4 tool_result 内容形态
- [ ] `content` 为字符串
- [ ] `content` 为嵌套 block 数组
- [ ] `content` 为 `document` block 数组
- [ ] 嵌套 block 中至少覆盖 `text` / `image` / `document`

### 2.5 tool_result 排序与格式约束
- [ ] `tool_result` 必须紧跟对应 assistant `tool_use` 之后
- [ ] user 消息中若同时有 `tool_result` 和 `text`，则 `tool_result` 必须排在前面
- [ ] 不合法顺序能被检测/修正/拒绝，而不是静默错配

### 2.6 Server tools（Anthropic 服务器执行）
- [ ] 能区分 **client tools** 和 **server tools**
- [ ] `web_search` 支持策略明确
- [ ] `web_fetch` 支持策略明确
- [ ] `pause_turn` 场景支持策略明确
- [ ] server-tool 继续对话逻辑支持（把 assistant 响应原样送回继续）

检查备注：
- 官方文档明确：client tool 需要你执行并回送 `tool_result`；server tool 由 Anthropic 服务器执行，可能返回 `pause_turn`。

---

## 3. Extended Thinking 覆盖

### 3.1 请求侧
- [ ] 支持 `thinking: {"type":"enabled","budget_tokens":...}`
- [ ] thinking 开启/关闭的映射策略明确
- [ ] tool use + thinking 联合使用时，限制符合官方说明

### 3.2 兼容性规则
- [ ] 当启用扩展思考时，仅允许 `tool_choice:auto` 或 `tool_choice:none`
- [ ] 对 `tool_choice:any` / `tool_choice:tool` 的不兼容场景有明确处理

### 3.3 历史回传 / round-trip
- [ ] 上一轮 assistant `thinking` block 回传时结构合法
- [ ] `signature` 原样保存/透传，不做业务解释
- [ ] 支持在需要时把 thinking block 回送给 Anthropic API

### 3.4 流式 thinking
- [ ] 正确处理 `thinking_delta`
- [ ] 正确处理 `signature_delta`
- [ ] `signature_delta` 出现在 `content_block_stop` 前的流式时序能正确还原

检查备注：
- 官方文档强调：`signature` 是不透明字段，只用于校验，不应自行解析。

---

## 4. Streaming / SSE 覆盖

### 4.1 基本事件序列
- [ ] `message_start`
- [ ] `content_block_start`
- [ ] `content_block_delta`
- [ ] `content_block_stop`
- [ ] `message_delta`
- [ ] `message_stop`
- [ ] `ping`
- [ ] `error`

### 4.2 delta 类型
- [ ] `text_delta`
- [ ] `input_json_delta`
- [ ] `thinking_delta`
- [ ] `signature_delta`
- [ ] `citations_delta`（如果你要覆盖引用）

### 4.3 tool streaming
- [ ] tool 参数流式增量拼装正确
- [ ] `eager_input_streaming` 场景支持策略明确
- [ ] 工具参数未完成时不应提前错误触发执行

### 4.4 stop_reason 在流中的位置
- [ ] `message_start` 中 `stop_reason` 为 `null`
- [ ] `stop_reason` 仅从 `message_delta` 读取
- [ ] 不依赖其它事件里的 `stop_reason`

### 4.5 流中断恢复
- [ ] 对 Claude 4.6：采用“追加 user 继续提示”的恢复思路
- [ ] 对 Claude 4.5 及更早：恢复策略与官方说明一致
- [ ] 对含 `tool_use` / `thinking` 的流中断，有明确“不做部分恢复”的策略

---

## 5. Stop Reasons 覆盖

- [ ] `end_turn`
- [ ] `max_tokens`
- [ ] `stop_sequence`
- [ ] `tool_use`
- [ ] `pause_turn`
- [ ] `refusal`
- [ ] `model_context_window_exceeded`

### 5.1 细项检查
- [ ] `max_tokens` 截断后有继续生成策略
- [ ] `pause_turn` 时继续对话逻辑正确
- [ ] `refusal` 与 HTTP error 区分清楚
- [ ] `model_context_window_exceeded` 有明确处理分支
- [ ] 若需兼容旧模型，是否处理 `model-context-window-exceeded-2025-08-26` beta header

检查备注：
- 官方文档强调：`stop_reason` 是**成功响应**中的停止原因；4xx/5xx 是**失败请求**，两者不能混淆。

---

## 6. 图片输入（Vision）覆盖

### 6.1 图片来源类型
- [ ] `image` + `source.type=base64`
- [ ] `image` + `source.type=url`
- [ ] `image` + `source.type=file` + `file_id`

### 6.2 图片格式
- [ ] `image/jpeg`
- [ ] `image/png`
- [ ] `image/gif`
- [ ] `image/webp`

### 6.3 图片数量/尺寸/大小限制
- [ ] 单次请求最多 100 张图片
- [ ] API 单图最大 5MB
- [ ] 请求总大小受 32MB 限制
- [ ] 单图超过 `8000x8000` 会被拒绝
- [ ] 当单请求图片数 > 20 时，尺寸限制切换到 `2000x2000`

### 6.4 图片与文本混排
- [ ] 图片在前、文本在后时行为正常
- [ ] 多图 + 文本标签（如 `Image 1:` / `Image 2:`）行为正常
- [ ] 多轮会话中再次追加图片仍能正确传递

### 6.5 Files API 图片复用
- [ ] 若支持 `file_id`，则显式处理 `files-api-2025-04-14` beta 相关能力
- [ ] image file 上传后在消息中引用 `file_id` 行为正确

---

## 7. PDF / 文档覆盖

### 7.1 PDF 输入来源
- [ ] `document` + `source.type=url`
- [ ] `document` + `source.type=base64`
- [ ] `document` + `source.type=file` + `file_id`

### 7.2 PDF 限制
- [ ] 每次请求最大 100 页
- [ ] 总请求大小最大 32MB
- [ ] 仅标准 PDF；加密/带密码 PDF 的处理策略明确

### 7.3 PDF 与文本顺序
- [ ] 文档在前、文本问题在后
- [ ] 文档与文本混排不乱序

### 7.4 PDF 理解相关能力
- [ ] 同时覆盖文本提取与视觉理解预期
- [ ] 若产品面支持引用，检查 PDF 引用定位是否保真（`page_location`）

### 7.5 普通文件 / 非 PDF 文件
- [ ] 对 `.csv` / `.md` / `.docx` / `.xlsx` 等非 `document` block 官方不直接支持的格式，策略明确
- [ ] 官方建议路线：先转纯文本再放进消息内容
- [ ] 含图片的 `.docx` 若要保留视觉理解，是否先转 PDF

### 7.6 Files API 文档复用
- [ ] 若支持 `file_id`，则文档上传 / 引用路径明确
- [ ] 是否支持 `title` / `context` / `citations` 之类文档附加字段，策略明确

检查备注：
- 官方文档里“普通文档”不要等同于“所有文件都能直接当 `document` block 发”。非 PDF 办公文件通常需要先转文本，或转 PDF。

---

## 8. Citations（如需覆盖）

- [ ] plain text 引用位置：`char_location`
- [ ] PDF 引用位置：`page_location`
- [ ] custom content 引用位置：`content_block_location`
- [ ] 流式 `citations_delta` 支持策略明确
- [ ] 文档引用开关 / 结果结构保持不丢字段

---

## 9. Files API（如转换器支持 `file_id`）

- [ ] 文件上传后在 `Messages` 中用 `file_id` 引用
- [ ] 图片 `file_id` 路线
- [ ] PDF `file_id` 路线
- [ ] 文件删除 / 找不到 / 类型不匹配 / 超限等错误处理策略明确
- [ ] 是否处理 Files API beta 特性与 header 要求

检查备注：
- 官方文档说明：Files API 处于 beta；`file_id` 能否用，取决于对应模型是否支持该文件类型。

---

## 10. MCP Connector（Messages API 侧）覆盖

### 10.1 请求结构
- [ ] `mcp_servers[]`
- [ ] `mcp_servers[].type="url"`
- [ ] `mcp_servers[].url`
- [ ] `mcp_servers[].name`
- [ ] `mcp_servers[].authorization_token`
- [ ] `tools[]` 中的 `mcp_toolset`
- [ ] `mcp_toolset.mcp_server_name`
- [ ] `default_config.enabled`
- [ ] `default_config.defer_loading`
- [ ] `configs.<tool>.enabled`
- [ ] `configs.<tool>.defer_loading`

### 10.2 验证规则
- [ ] 每个 `mcp_server` 都必须被恰好一个 `mcp_toolset` 引用
- [ ] `mcp_server_name` 必须匹配已定义服务器
- [ ] 多服务器场景可正常工作

### 10.3 响应块类型
- [ ] `mcp_tool_use`
- [ ] `mcp_tool_result`
- [ ] `mcp_tool_result.is_error`

### 10.4 限制/前置条件
- [ ] 需要 beta 头：`anthropic-beta: mcp-client-2025-11-20`
- [ ] 只支持工具调用，不等同于完整 MCP 资源/提示/stdio 客户端
- [ ] 远程服务器必须经 HTTP 暴露（支持 Streamable HTTP / SSE）
- [ ] 本地 stdio 服务器不能直接走 Messages API MCP connector
- [ ] ZDR 不适用于该 beta 特性

---

## 11. Token Counting 覆盖

### 11.1 `/v1/messages/count_tokens`
- [ ] 独立支持 token counting 端点
- [ ] 与 `/v1/messages` 共享同样的结构化输入形态
- [ ] 支持 `system`
- [ ] 支持 `tools`
- [ ] 支持图片
- [ ] 支持 PDF
- [ ] 支持 thinking 场景

### 11.2 语义检查
- [ ] 明确其结果是 **estimate**，不是绝对精确值
- [ ] 不把 token counting 当成真实采样结果
- [ ] server tools 的 token count 仅适用于第一次 sampling call
- [ ] 以前 assistant turn 的 thinking block 不计入输入 token
- [ ] 当前 assistant turn 的 thinking 会计入输入 token

### 11.3 运行特性
- [ ] token counting 免费但受单独 RPM 限制
- [ ] 与 `/v1/messages` 的 rate limit 分开统计

---

## 12. Claude Code：Plan Mode 覆盖

> 这一节是 **Claude Code 官方行为兼容检查**，不是 Messages API 原生协议。

- [ ] 支持 Plan Mode 入口语义：`--permission-mode plan`
- [ ] 支持会话内切换 Plan Mode 的兼容预期
- [ ] Plan Mode 下优先执行只读分析，而不是直接改文件
- [ ] Plan Mode 下支持通过 `AskUserQuestion` 澄清需求的兼容预期
- [ ] 支持“先产出计划、后等用户确认”的交互节奏
- [ ] `permissions.defaultMode=plan` 这类设置影响有明确策略

检查备注：
- 官方文档把 Plan Mode 定义为“安全代码分析 / 只读探索 / 先规划再改动”的模式。

---

## 13. Claude Code：CLI 图片能力覆盖

> 这一节检查 Claude Code 侧“把图片带进会话”的兼容面。

- [ ] 支持通过图片路径引用图片
- [ ] 支持多图同轮输入的兼容预期
- [ ] 图片在 Claude Code 中进入会话后，转换层能保持其多模态语义
- [ ] 截图/设计稿/错误界面这类输入不会被错误降级为普通文本

检查备注：
- 官方 Claude Code 文档支持：拖拽、粘贴、路径引用图片；转换器至少要保证“图片仍然是图片语义”。

---

## 14. Claude Code：MCP 覆盖

### 14.1 CLI 管理命令
- [ ] `/mcp`
- [ ] `claude mcp add`
- [ ] `claude mcp list`
- [ ] `claude mcp get`
- [ ] `claude mcp remove`

### 14.2 传输类型
- [ ] `--transport http`
- [ ] `--transport sse`
- [ ] `--transport stdio`

### 14.3 MCP 资源 / 提示 / 搜索
- [ ] 资源引用：`@server:protocol://path`
- [ ] MCP prompts 转 slash commands：`/mcp__server__prompt`
- [ ] `ENABLE_TOOL_SEARCH` 相关行为有明确策略
- [ ] `MAX_MCP_OUTPUT_TOKENS` 超限预期有明确策略

### 14.4 认证 / 环境
- [ ] OAuth 远程 MCP 认证路径兼容预期明确
- [ ] `ENABLE_CLAUDEAI_MCP_SERVERS` 之类环境开关若涉及，有明确策略

检查备注：
- 官方 Claude Code MCP 文档里的“资源引用 / prompt 命令化 / stdio 服务器”不等同于 Messages API MCP connector；两套能力要分开检查。

---

## 15. Claude Code：Subagents 覆盖

- [ ] `/agents` 工作流兼容预期明确
- [ ] subagent 有独立 context window
- [ ] subagent 不继承主会话完整对话历史
- [ ] subagent 结果回到主会话时不会丢关键信息
- [ ] subagent 不能再生成 subagents（避免嵌套委托）
- [ ] 自定义 subagent frontmatter 关键字段策略明确：
  - [ ] `name`
  - [ ] `description`
  - [ ] `tools`
  - [ ] `disallowedTools`
  - [ ] `model`
  - [ ] `permissionMode`
  - [ ] `maxTurns`
  - [ ] `skills`
  - [ ] `mcpServers`
  - [ ] `hooks`
  - [ ] `memory`
  - [ ] `background`
  - [ ] `isolation`
- [ ] `background=true` / 后台 subagent 行为兼容预期明确
- [ ] `isolation: worktree` 行为兼容预期明确

检查备注：
- 官方文档明确区分 subagents 与 agent teams：subagents 更轻、更偏“只把结果带回主线程”。

---

## 16. Claude Code：Agent Teams / Teammate 覆盖

> 如果你说的“teammate”指官方 Claude Code 能力，这里应按 **Agent Teams** 检查；它是官方公开文档中的“teammates”概念。

### 16.1 启用与前置条件
- [ ] Agent Teams 默认关闭
- [ ] 需要 `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1`
- [ ] 创建 team 前需要用户批准

### 16.2 核心机制
- [ ] team lead / teammates / task list / mailbox 语义不丢失
- [ ] teammates 可直接互相通信
- [ ] 共享 task list 的状态语义保真：`pending` / `in progress` / `completed`
- [ ] 任务依赖与解锁语义保真

### 16.3 teammate 操作
- [ ] 可以直接给 teammate 发消息
- [ ] 可以要求 teammate shutdown
- [ ] cleanup 只能由 lead 执行
- [ ] cleanup 前若仍有 active teammates，行为符合官方预期

### 16.4 hooks / 事件
- [ ] `TeammateIdle` 相关兼容预期明确
- [ ] `TaskCompleted` 相关兼容预期明确

### 16.5 与 Subagents 的边界
- [ ] 能区分 subagents 与 agent teams
- [ ] 知道 agent teams 成本更高、上下文更独立、适合需要互相讨论/协调的并行任务

检查备注：
- 官方文档里 teammate 是 agent teams 体系的一部分，不是普通 subagent。

---

## 17. Worktree / 并行隔离覆盖

- [ ] Claude Code `--worktree` / `-w` 语义兼容预期明确
- [ ] 自动命名 worktree 与显式命名 worktree 的兼容预期明确
- [ ] 无改动自动清理 / 有改动询问保留或删除 的语义明确
- [ ] subagent `isolation: worktree` 行为与普通 subagent 行为区分明确
- [ ] 非 git / 自定义 VCS hook 场景策略明确

检查备注：
- 官方文档明确：worktree 是隔离选项，不是所有 subagent 的默认前提。

---

## 18. Header / Beta / 版本兼容检查

- [ ] `anthropic-version` 请求头处理明确
- [ ] Files API beta：`files-api-2025-04-14`（如支持 `file_id`）
- [ ] MCP connector beta：`mcp-client-2025-11-20`（如支持 Messages API MCP）
- [ ] `model-context-window-exceeded-2025-08-26`（如要兼容旧模型 stop_reason）
- [ ] `interleaved-thinking-2025-05-14`（如要兼容相关思考/工具联合能力）

---

## 19. 负向 / 错误路径检查

- [ ] 非法 `tool_result` 顺序返回明确错误，而不是静默错位
- [ ] 非法 tool name / schema 能被拒绝或修正
- [ ] 超大图片 / 超多图片 / 超大 PDF 的错误路径明确
- [ ] 无效 `file_id` / 文件类型不匹配的错误路径明确
- [ ] MCP server 缺失 / 名称不匹配 / 多重引用冲突错误路径明确
- [ ] 流式中断时不会把半截 `tool_use` / `thinking` 当成完整块
- [ ] `pause_turn` 没被错误当成最终回答
- [ ] `refusal` 没被错误当成空响应或 HTTP 错误

---

## 20. 建议的回归测试矩阵

### 最小必测集
- [ ] 纯文本单轮
- [ ] 纯文本多轮
- [ ] client tool 单工具调用
- [ ] client tool 多工具 / 并行工具调用
- [ ] `tool_choice=auto`
- [ ] `tool_choice=any`
- [ ] `tool_choice=tool`
- [ ] `tool_choice=none`
- [ ] thinking 非流式
- [ ] thinking 流式
- [ ] 单图输入
- [ ] 多图输入
- [ ] PDF(base64)
- [ ] PDF(url)
- [ ] `file_id` 图片
- [ ] `file_id` PDF
- [ ] `/count_tokens` + tools
- [ ] `/count_tokens` + image
- [ ] `/count_tokens` + PDF
- [ ] Messages API MCP connector（单 server）
- [ ] Messages API MCP connector（多 server）
- [ ] server tool + `pause_turn`
- [ ] stop reasons 全量覆盖
- [ ] Claude Code Plan Mode
- [ ] Claude Code subagent
- [ ] Claude Code teammate / agent team
- [ ] worktree 隔离

### 可选增强集
- [ ] citations
- [ ] MCP resources `@server:...`
- [ ] MCP prompts `/mcp__...`
- [ ] `ENABLE_TOOL_SEARCH`
- [ ] 流式 tool eager input
- [ ] 拒绝 / 安全中止流

---

## 21. 执行记录模板

### Case
- 名称：
- 官方依据：
- 输入：
- 预期：
- 实际：
- 结论：`通过 / 有差异 / 不适用`
- 备注：

---

## 22. 最后提醒

- 这份清单是**官方能力面 checklist**，不是你当前项目现状。
- 如果你接下来要做的是“对照你仓库实现逐项打勾”，下一步就应该把这份文档再改成 **两列版**：
  1. 官方要求
  2. 当前实现/测试证据
- 如果你要，我下一轮可以继续：**在不看业务代码逻辑细节的前提下，只按公开接口和测试入口，把这份清单改成“可执行测试版”**（每项附 curl / Claude Code 复现步骤）。
