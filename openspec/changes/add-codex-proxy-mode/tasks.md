## 1. 配置模型与迁移

- [x] 1.1 在前端 `ProxyConfigV2` 中新增 Codex 档位配置类型，包含目标地址、API 密钥、端点列表、选中端点、代理模式和转换器字段
- [x] 1.2 在 Tauri `ProxyConfig` 中新增对应的 `codexConfig` 结构，并为旧配置文件提供默认值
- [x] 1.3 调整配置加载、保存、导入和导出逻辑，确保 Claude 顶层配置与 Codex 子配置同时保留
- [x] 1.4 添加配置迁移测试或等价验证，覆盖旧配置加载后 Claude 配置不变且 Codex 配置补齐默认值

## 2. 运行时路由隔离

- [x] 2.1 将 Rust 运行时配置扩展为 Claude route 与 Codex route 两套独立目标配置
- [x] 2.2 调整启动代理和热更新构建逻辑，使每次提交完整 Claude/Codex route 配置
- [x] 2.3 在请求处理入口识别 `/codex` 前缀，剥离前缀后使用 Codex route 处理请求
- [x] 2.4 保持 `/messages`、`/v1/messages`、`/messages/count_tokens`、`/v1/messages/count_tokens` 使用 Claude route
- [x] 2.5 添加后端回归测试，覆盖 `/codex/v1/messages`、`/codex/messages`、`/codex/v1/messages/count_tokens` 使用 Codex 配置，普通 Claude 路径不受影响

## 3. 桌面端界面

- [x] 3.1 在主界面标题区域新增 Claude / Codex 档位切换控件
- [x] 3.2 将现有完整表单绑定到 Claude 档位，并保持当前 Claude UI 行为不变
- [x] 3.3 为 Codex 档位实现精简单模型表单，只展示端口、代理模式、目标地址和 Codex API 密钥，转换器固定为 Codex 透传
- [x] 3.4 确保 Codex 档位编辑 endpointOptions 时只写入 Codex 配置，不修改 Claude endpointOptions
- [x] 3.5 调整配置指南文案，使 Claude 档位继续显示 Claude Code 配置，Codex 档位显示 `http://localhost:<port>/codex/v1`

## 4. 验证

- [x] 4.1 运行前端类型检查或构建，确认新增配置类型和 UI 绑定没有类型错误
- [x] 4.2 运行 Rust 测试，确认路由选择、热更新和旧配置迁移行为通过
- [ ] 4.3 手动验证启动代理后 Claude 与 Codex 两个入口共用同一个运行状态，停止代理后两个入口同时停止
- [ ] 4.4 手动验证 Claude 与 Codex 目标地址互不生效，切换 UI 档位和重启应用后仍保持隔离
