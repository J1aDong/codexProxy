# Design: Claude 模型 Reasoning Effort 映射

## Context

Codex API 支持 `reasoning` 参数来控制推理强度，可选值包括：
- `xhigh` - 最高推理强度
- `high` - 高推理强度  
- `medium` - 中等推理强度
- `low` - 低推理强度

当前实现中，所有 Claude 模型都被硬编码为 `xhigh`，用户无法自定义。

### 约束条件
- 必须保持向后兼容（默认行为不变）
- 配置需要持久化到本地文件系统
- UI 需要支持中英文双语

## Goals / Non-Goals

### Goals
- 允许用户为不同 Claude 模型配置不同的 reasoning effort
- 提供合理的默认映射（opus→xhigh, sonnet→medium, haiku→low）
- 配置持久化，重启后恢复
- 提供"恢复默认"功能

### Non-Goals
- 不支持自定义模型名称（仅支持 opus/sonnet/haiku）
- 不支持运行时动态切换（需要重启代理生效）

## Decisions

### 1. 数据结构设计

```rust
/// Reasoning effort 级别枚举（类型安全）
#[derive(Debug, Serialize, Deserialize, Clone, Copy, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    Xhigh,
    High,
    #[default]
    Medium,
    Low,
}

impl ReasoningEffort {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReasoningEffort::Xhigh => "xhigh",
            ReasoningEffort::High => "high",
            ReasoningEffort::Medium => "medium",
            ReasoningEffort::Low => "low",
        }
    }
}

/// 模型到 reasoning effort 的映射配置
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ReasoningEffortMapping {
    #[serde(default = "default_opus")]
    pub opus: ReasoningEffort,      // 默认 Xhigh
    #[serde(default = "default_sonnet")]
    pub sonnet: ReasoningEffort,    // 默认 Medium
    #[serde(default = "default_haiku")]
    pub haiku: ReasoningEffort,     // 默认 Low
}

fn default_opus() -> ReasoningEffort { ReasoningEffort::Xhigh }
fn default_sonnet() -> ReasoningEffort { ReasoningEffort::Medium }
fn default_haiku() -> ReasoningEffort { ReasoningEffort::Low }

impl Default for ReasoningEffortMapping {
    fn default() -> Self {
        Self {
            opus: default_opus(),
            sonnet: default_sonnet(),
            haiku: default_haiku(),
        }
    }
}
```

**理由**: 
- 使用枚举提供类型安全，避免无效值
- `#[serde(rename_all = "lowercase")]` 确保 JSON 序列化为小写字符串
- `#[serde(default)]` 确保向后兼容，旧配置文件自动使用默认值

### 2. 配置存储位置

复用现有的 `proxy-config.json` 文件，新增 `reasoningEffortMapping` 字段：

```json
{
  "port": 8889,
  "targetUrl": "https://...",
  "apiKey": "",
  "reasoningEffortMapping": {
    "opus": "xhigh",
    "sonnet": "medium",
    "haiku": "low"
  }
}
```

**理由**: 
- 复用现有配置机制，减少代码改动
- 用户配置集中管理

### 3. 模型识别逻辑

```rust
/// 根据模型名称获取对应的 reasoning effort
/// 匹配优先级：opus > sonnet > haiku
fn get_reasoning_effort(model: &str, mapping: &ReasoningEffortMapping) -> ReasoningEffort {
    let model_lower = model.to_lowercase();
    // 优先级：opus > sonnet > haiku
    // 如果模型名同时包含多个关键字，按此顺序匹配
    if model_lower.contains("opus") {
        mapping.opus
    } else if model_lower.contains("sonnet") {
        mapping.sonnet
    } else if model_lower.contains("haiku") {
        mapping.haiku
    } else {
        // 未知模型使用 medium 作为安全默认值
        log::warn!("Unknown model '{}', using default 'medium'", model);
        ReasoningEffort::Medium
    }
}
```

**理由**: 
- 使用 `contains` 匹配，兼容各种模型名称格式（如 `claude-3-opus-20240229`、`claude-3-5-sonnet-20241022`）
- 明确匹配优先级，避免歧义
- 未知模型记录警告日志便于调试

### 4. UI 设计

在现有配置卡片中新增一个折叠区域：

```
┌─────────────────────────────────────────┐
│ 推理强度配置 (Reasoning Effort)          │
├─────────────────────────────────────────┤
│ Opus:   [xhigh ▼]                       │
│ Sonnet: [medium ▼]                      │
│ Haiku:  [low ▼]                         │
└─────────────────────────────────────────┘
```

每个下拉框选项：`xhigh`, `high`, `medium`, `low`

## Risks / Trade-offs

| 风险 | 缓解措施 |
|------|----------|
| 用户配置错误的 effort 值 | UI 使用下拉选择器限制可选值 |
| 配置文件损坏 | 加载失败时使用默认值 |
| 新模型不在映射中 | 使用 `medium` 作为默认值 |

## Migration Plan

1. 新增配置字段，旧配置文件自动使用默认值
2. 无需数据迁移
3. 向后兼容，无破坏性变更

## Open Questions

- 是否需要支持更细粒度的模型版本映射？（当前决定：不需要，按模型系列映射即可）
