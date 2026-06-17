# 配置指南：从"复制粘贴"改为"一键写入文件"

## Context

当前 `GuideSection.vue` 展示 Claude Code / Codex 的配置 JSON，用户需手动复制到 `~/.claude/settings.json`。目标：**点击按钮直接写入对应配置文件**，并增加 Codex 的同类支持。

## 已确认的关键事实

### Claude Code 配置
- 路径：`~/.claude/settings.json`（JSON）
- 需写入字段：`env.ANTHROPIC_BASE_URL`、`env.ANTHROPIC_AUTH_TOKEN`、`forceLoginMethod`
- **合并策略**：merge（保留用户已有的其他字段，如 `permissions`）
- **token 值**：固定占位字符串（与当前 UI 提示一致，因为 proxy 端可配置覆盖）

### Codex CLI 配置（已查证官方文档）
- 路径：`~/.codex/config.toml`（**TOML 格式**，非 JSON）
- 关键字段：
  - `model_provider = "codex-proxy"`（指定使用自定义 provider）
  - `[model_providers.codex-proxy]` 表：
    - `name = "Codex Proxy"`
    - `base_url = "http://localhost:{port}/codex/v1"`
    - `env_key = "OPENAI_API_KEY"`（Codex 会读这个环境变量作为 key）
    - `wire_api = "responses"`
- Codex 端点路径已确认：`/codex/v1`（见 `server.rs:9150`）

## 方案概览

| 改动 | 文件 | 说明 |
|------|------|------|
| 新增 Tauri 命令 | `proxy.rs` + `main.rs` | `apply_claude_config`、`apply_codex_config` |
| 新增桥接函数 | `configBridge.ts` | 前端调用新命令 |
| 重构 UI | `GuideSection.vue` | 复制按钮 → "一键写入配置" 按钮 + 结果反馈 |
| 新增 i18n | `zh.ts`, `en.ts` | 新增文案 |

## 详细步骤

### 1. Tauri 后端：新增两个命令（`proxy.rs`）

**`apply_claude_config(port, auth_token) -> Result<String, String>`**
- 路径：`~/.claude/settings.json`
- 逻辑：
  1. 读取现有文件（不存在则 `{}`）
  2. 解析为 `serde_json::Value`
  3. 合并：设置 `env.ANTHROPIC_BASE_URL`、`env.ANTHROPIC_AUTH_TOKEN`、`forceLoginMethod = "claudeai"`
  4. **保留** `permissions` 等其他字段不动
  5. 写回（pretty print，2 空格缩进）
- 返回：成功返回写入路径字符串

**`apply_codex_config(port) -> Result<String, String>`**
- 路径：`~/.codex/config.toml`
- 逻辑：
  1. 读取现有文件（不存在则空字符串）
  2. 解析为 TOML（用 `toml` crate）
  3. 设置 `model_provider = "codex-proxy"`
  4. 在 `[model_providers.codex-proxy]` 表中写入 `name`、`base_url`、`env_key`、`wire_api`
  5. **保留** 其他字段不动
  6. 写回
- 返回：成功返回写入路径字符串

**依赖**：`proxy.rs` 已有 `serde_json`，需确认 `toml` crate 是否已引入（`Cargo.toml` 中 main crate 已用 toml，Tauri 端需检查）。

**注册命令**：在 `main.rs` 的 `invoke_handler!` 中添加两个新命令。

### 2. 前端 Bridge 层（`configBridge.ts`）

```typescript
export const applyClaudeConfig = (port: number, authToken: string): Promise<string> =>
  invoke<string>('apply_claude_config', { port, authToken })

export const applyCodexConfig = (port: number): Promise<string> =>
  invoke<string>('apply_codex_config', { port })
```

### 3. GuideSection.vue UI 改造

**保留**：代码块预览（供用户参考）
**新增**：
- 右上角按钮区：复制按钮 + **"一键写入配置"** 按钮（主操作）
- 点击"一键写入配置"后：
  - Claude 模式：调用 `applyClaudeConfig(port, 'proxy-configured')`（token 用固定占位字符串）
  - Codex 模式：调用 `applyCodexConfig(port)`
- 反馈状态：
  - loading：按钮禁用 + "写入中..."
  - 成功：绿色 ✓ + "已写入 {path}"（2 秒后恢复）
  - 失败：红色提示 + 错误信息（用 Dialog 或 inline 文本）

### 4. i18n 新增条目

```typescript
// zh.ts
applyConfig: '一键写入配置',
applyConfigSuccess: '已写入 {path}',
applyConfigFailed: '写入失败: {error}',
applyingConfig: '写入中...',

// en.ts
applyConfig: 'Apply Config',
applyConfigSuccess: 'Written to {path}',
applyConfigFailed: 'Write failed: {error}',
applyingConfig: 'Writing...',
```

## 验证方式

1. `cd fronted-tauri && npm run tauri dev` 启动
2. Claude 模式下点击"一键写入配置"，检查 `~/.claude/settings.json` 是否被正确合并写入
3. Codex 模式下点击"一键写入配置"，检查 `~/.codex/config.toml` 是否被正确合并写入
4. 验证已有配置（如 `permissions`、其他 model_providers）不被覆盖
5. 验证失败场景（如目录不存在）的错误提示

## 不做的事

- 不改动后端核心代理逻辑（`main/src/`）
- 不改动负载均衡、转换器等已有功能
- 不引入新的 npm 依赖
- 不自动备份原文件（merge 策略已足够安全；如需可后续加）
