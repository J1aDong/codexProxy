# Tasks: use-litellm-proxy

## 状态：分析完成 - 不建议采用

基于可行性分析，LiteLLM 不适合当前场景。以下是如果要强行使用 LiteLLM 需要的任务（仅供参考）：

---

## 如果选择使用 LiteLLM（不推荐）

### Phase 1: 环境准备
- [ ] 安装 Python 3.8+ 环境
- [ ] 安装 LiteLLM: `pip install litellm`
- [ ] 创建 `litellm_config.yaml` 配置文件

### Phase 2: 自定义 Provider 开发
- [ ] 创建 `codex_provider.py` 实现 `CustomLLM` 基类
- [ ] 实现 `completion()` 方法 - 请求格式转换
- [ ] 实现 `streaming()` 方法 - SSE 响应解析
- [ ] 实现工具调用格式转换
- [ ] 实现图片/文档内容处理
- [ ] 注入 Codex instructions 模板

### Phase 3: 集成测试
- [ ] 测试基本文本对话
- [ ] 测试工具调用流程
- [ ] 测试流式响应
- [ ] 测试多轮对话
- [ ] 性能对比测试

### Phase 4: 部署
- [ ] 编写 Docker 配置
- [ ] 更新启动脚本
- [ ] 更新文档

**预估工作量**：与重写当前 JS 实现相当，但增加了 Python 依赖

---

## 推荐替代方案

### 方案 A: 重构现有代码（推荐）

- [ ] 将 `codex-proxy-anthropic.js` 拆分为模块：
  - `transform-request.js` - 请求转换
  - `transform-response.js` - 响应转换
  - `transform-tools.js` - 工具格式转换
  - `server.js` - HTTP 服务器
- [ ] 添加 JSDoc 类型注释
- [ ] 添加单元测试
- [ ] 添加错误处理和日志

### 方案 B: TypeScript 重写

- [ ] 初始化 TypeScript 项目
- [ ] 定义类型接口 (Anthropic/Codex)
- [ ] 迁移转换逻辑
- [ ] 添加测试

---

## 结论

当前 `codex-proxy-anthropic.js` 已经是最简方案。如需改进，建议走重构路线而非引入 LiteLLM。
