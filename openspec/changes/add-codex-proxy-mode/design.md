## Context

当前桌面端只有一套主要配置：端口、代理模式、目标地址、API 密钥、转换器、模型映射和负载均衡配置都会进入同一个 `ProxyConfig`。Rust 侧 `ProxyServer` 的运行时快照也只有一套 `target_url`、`api_key`、`converter` 和 `TransformContext`，所有 `/messages`、`/v1/messages`、`/messages/count_tokens`、`/v1/messages/count_tokens` 请求都会使用这套配置。

这次变更要解决两个问题：

- Claude Code 与 Codex 客户端需要同时使用同一个本地代理进程。
- Codex 的目标地址配置不能复用或污染 Claude 档位，否则用户在两个客户端之间切换时仍然会出现配置混淆。

约束：

- Claude 档位必须维持当前行为，包括现有 `/v1/messages` 兼容入口和现有配置字段的向后兼容。
- `/codex` 是 Codex 档位的本地入口前缀，不应改变 Claude 现有入口；`/codex/v1/**` 同时作为 Codex/OpenAI 原生 HTTP 透传入口。
- 启动代理 / 停止代理是进程级动作，不随 UI 当前档位变化。
- `main/src/transform/unified.rs` 继续只承载协议无关抽象，Codex 档位隔离不应通过污染统一输入层实现。

## Goals / Non-Goals

**Goals:**

- 在 UI 顶部新增 Claude / Codex 档位切换。
- Claude 档位继续展示并使用当前完整配置。
- Codex 档位展示精简的单模型配置界面，只保留端口、代理模式、目标地址和 Codex API 密钥；转换器固定为 Codex 透传，不在 Codex 档位暴露选择。
- 将 Codex 档位的目标地址、API 密钥、端点列表和当前选择与 Claude 档位分开持久化。
- 让同一个运行中的代理同时处理 Claude 入口和 Codex 入口。
- 请求路径以 `/codex` 开头时选择 Codex 配置；其他现有 Claude 入口继续选择 Claude 配置。

**Non-Goals:**

- 不重新设计现有负载均衡能力。
- 不改变 Claude 档位现有模型映射、转换器和配置指南语义。
- 不把 Codex 专属配置放入 `UnifiedChatRequest`。
- 不引入新的外部依赖。
- 不在本变更里实现跨 Claude/Codex 的配置同步。

## Decisions

1. **配置模型采用“双配置组 + 旧字段兼容”**

   保留现有顶层 `ProxyConfig` 字段作为 Claude 档位配置来源，新增 `codexConfig` 子对象保存 Codex 档位的目标地址、API 密钥、端点列表、当前选中端点、代理模式和转换器。这样旧配置文件可以继续按 Claude 配置加载，新增字段缺失时用 Codex 默认值补齐。

   备选方案是把 Claude 也迁移到 `claudeConfig` 子对象。这个方案结构更对称，但会扩大迁移范围，也更容易破坏导入导出和热更新兼容性。本次优先选择低风险增量方案。

2. **UI 当前档位只决定编辑哪组配置，不决定代理是否运行**

   `activeClientMode = "claude" | "codex"` 只控制界面展示和表单读写目标。启动、停止、运行状态、端口占用检测仍然全局共享。

   这样符合用户预期：点击“启动代理”后，Claude Code 与 Codex 都可以同时使用；切换 UI 档位只是编辑另一套配置。

3. **Rust 运行时快照保存 Claude 与 Codex 两套路由配置**

   `RuntimeConfigUpdate` 和 `RuntimeConfigState` 需要扩展为包含 `claude_route` 与 `codex_route`。现有字段可以先映射为 Claude route，Codex route 来自新增配置。请求进入 `handle_request` 后，先根据 path 判断是否为 Codex 前缀，再选择对应 route 的 `target_url`、`api_key`、`converter` 和上下文。

   备选方案是在前端切换档位时热更新整个代理运行时。这个方案无法满足同时使用，因为同一时间只能有一套生效配置。

4. **`/codex` 作为路径前缀，不替代现有 Claude 路径**

   Codex 客户端配置本地 base URL 为 `http://localhost:<port>/codex/v1`。服务端识别 `/codex` 前缀后使用 Codex route，并对后续路径继续按现有请求处理规则匹配。

   `/codex/v1/messages` 仍保留 Anthropic-compatible 兼容入口；其他 `/codex/v1/**` 路径作为原生 HTTP 透传，保留 method、query、headers 和 body，并使用 Codex 档位的目标地址与 API 密钥转发。

5. **导入导出和热更新必须包含 Codex 配置**

   保存配置、导入配置和运行中热更新都使用同一个完整 `ProxyConfig`。当运行中修改 Codex 档位目标地址时，只更新 Codex route；当修改 Claude 档位目标地址时，只更新 Claude route。端口变化仍然需要重启。

## Risks / Trade-offs

- [Risk] 旧配置文件没有 Codex 字段，加载后可能出现空目标地址 → Mitigation：为 `codexConfig` 提供默认端点、默认选中 ID 和空密钥，并在迁移时补齐。
- [Risk] `/codex` 前缀与现有路径匹配规则混在一起，可能误判 404 → Mitigation：在进入现有 `is_messages` / `is_count_tokens` 判断前统一剥离前缀，并为 `/codex/v1/messages`、`/codex/messages`、`/codex/v1/messages/count_tokens`、`/codex/v1/models` 和 `/codex/v1/responses` 添加回归测试。
- [Risk] UI 档位切换时 accidentally 写回另一套 endpointOptions → Mitigation：封装当前档位的 endpoint getter/setter，避免直接复用单一 `form.endpointOptions` 写入两处。
- [Risk] 运行时热更新只更新当前 UI 档位，导致另一档位保存但未生效 → Mitigation：`buildProxyConfig` 始终提交完整配置，Tauri 侧每次构建包含两套路由的 `RuntimeConfigUpdate`。
- [Risk] 精简 Codex UI 与现有完整表单共享代码后出现隐藏字段残留 → Mitigation：模板层按 `activeClientMode` 控制展示，保存层只序列化 Codex 允许字段到 `codexConfig`。

## Migration Plan

1. 加载旧配置时，将现有顶层字段视为 Claude 配置，并为 `codexConfig` 填充默认单模型配置。
2. 保存配置时继续保留现有顶层字段，同时写入新的 `codexConfig`。
3. 热更新时生成 Claude route 与 Codex route，端口不变则无需重启。
4. 如需回滚，旧版本会忽略未知的 `codexConfig` 字段，Claude 档位仍可按原字段工作。

## Open Questions

- 已确定 Codex 客户端需要原生 OpenAI/Codex HTTP 透传能力；`/codex/v1/**` 中除 Anthropic-compatible messages 兼容路径外均走原生透传。
