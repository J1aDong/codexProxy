# Task Plan: Vue AI学习助手实施计划

## Goal
构建一个 Vue 纯前端 AI 学习助手：左侧话题列表、中间 Markdown 学习文档、右侧聊天面板；通过桥接层调用 Claude Code；采用 TDD、小步可调试、真实用例；首版仅本地持久化。

## Phases
- [x] Phase 1: Plan and setup
- [ ] Phase 2: Research/gather information
- [ ] Phase 3: Execute/build plan document
- [ ] Phase 4: Review and deliver

## Key Questions
1. Vue 前端基座应优先选择哪个可二开的项目？
2. Claude Code 网页桥接应采用哪种参考实现与安全边界？
3. 如何把需求切成严格 TDD 的最小实现单元并保证真实用例？

## Decisions Made
- 前端技术栈: Vue 纯前端。理由：实现成本更低，符合用户偏好。
- 许可证偏好: MIT/Apache-2.0 优先。理由：便于后续商用与二开。
- 持久化策略: 首版仅本地持久化。理由：降低复杂度，先打通主链路。
- 开发方式: 强制 TDD + 小步迭代 + 真实用例。理由：可调试、可验证、降低返工。

## Errors Encountered
- 暂无

## Status
**Currently in Phase 2** - 正在整理候选项目与桥接方案的研究结论并写入 notes.md。
