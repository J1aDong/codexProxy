export default {
    // Status
    statusRunning: '代理运行中',
    statusStopped: '代理已停止',

    // Header
    title: 'Codex 代理',

    // Config Card
    port: '端口',
    codexModel: 'Codex 模型',
    targetUrl: '目标地址',
    selectTargetUrl: '选择目标地址',
    edit: '编辑',
    apiKey: 'Codex API 密钥',
    apiKeyPlaceholder: '选填 - 将覆盖客户端提供的密钥',
    apiKeyTip: '如果在此处配置，您可以在 Claude Code 中使用任意随机字符串作为 API 密钥。',
    modelRecommended: 'GPT-5.3-Codex (推荐)',
    converter: '转换器',
    converterCodex: 'Codex',
    converterGemini: 'Gemini',
    geminiModel: 'Gemini 模型',
    reasoningEffort: '推理强度配置',
    effortLevel: '推理强度',
    geminiReasoningEffort: 'Gemini 推理强度',
    reasoningEffortTip: '为不同的 Claude 模型系列设置默认推理强度级别。',
    geminiReasoningEffortTip: 'Gemini 转换器使用单一推理强度。',
    restoreDefaults: '恢复默认',
    startProxy: '启动代理',
    stopProxy: '停止代理',

    // Endpoint Dialog
    addEndpoint: '添加地址',
    editEndpoint: '编辑地址',
    endpointAlias: '别名',
    endpointAliasPlaceholder: '例如：自建节点',
    endpointUrl: '地址',
    endpointApiKey: '密钥',

    // Guide
    guideTitle: '配置指南',
    guideDesc: '请将以下内容添加到您的 Claude Code 配置文件：',
    guideTokenHint: '替换为真实的key或者假如在proxy页面中配置了则任意字符串',
    copy: '复制',
    copied: '已复制',

    // Logs
    logsTitle: '系统日志',
    clearLogs: '清除日志',
    noLogs: '暂无日志...',

    // Menu
    menuPromptSettings: '提示词设置',
    menuAdvancedSettings: '高级设置',
    menuAbout: '关于',
    menuLogs: '日志',
    menuImportExport: '导入/导出',

    // Advanced Settings
    advancedSettingsTitle: '高级设置',
    advancedMaxConcurrencyLabel: '最大并发请求数',
    advancedMaxConcurrencyTip: '0 或留空表示不限制。建议从 2-4 开始逐步调高。',
    advancedMaxConcurrencyPlaceholder: '0 = 不限制',
    advancedIgnoreProbeLabel: '忽略启动探测请求（foo/count）',
    advancedIgnoreProbeTip: '仅对短探测请求本地快速返回，减少无意义上游调用与 429 风险。',
    advancedCountTokensFallbackLabel: 'count_tokens 失败时允许估算回退',
    advancedCountTokensFallbackTip: '开启后更稳定；关闭后会在上游失败时直接返回错误，便于排查。',
    advancedCodexCapabilityPresetLabel: 'Codex 模型-强度预设(JSON)',
    advancedCodexCapabilityPresetTip: '键为模型名，值为允许强度数组（low/medium/high/xhigh）。保存时会自动规范化。',
    advancedGeminiModelPresetLabel: 'Gemini 模型预设(JSON)',
    advancedGeminiModelPresetTip: '值为模型名称数组，用于 Gemini 转换器下拉选项。保存时会去重并过滤空值。',
    advancedCapabilityJsonError: 'JSON 格式错误，请检查后重试。',
    advancedGeminiPresetJsonError: 'Gemini 模型预设 JSON 格式错误，请检查后重试。',
    advancedSettingsRiskTip: '注意：忽略探测请求可能影响极少数客户端的能力检测行为。',

    // About
    aboutTitle: '关于',
    versionLabel: '版本',
    appName: 'Codex Proxy',
    updateIdle: '点击"前往 Release 页面"检查更新',
    updateChecking: '正在检查更新...',
    updateLatest: '当前已是最新版本',
    updateAvailable: '发现新版本',
    updateFailed: '检查更新失败',
    updateRateLimited: '检查更新失败（GitHub 限流）',
    goToReleases: '前往 Release 页面',

    // Settings
    settingsTitle: '设置',
    skillInjection: '技能注入配置',
    skillInjectionTip: '当使用 Skill 时，将此提示词注入到 User Message 中。留空则不注入。',
    skillInjectionPlaceholder: '例如：如果依赖缺失，请自动安装...',
    useDefaultPrompt: '使用默认提示词',

    // Import/Export
    importExportTitle: '配置导入/导出',
    exportConfig: '导出配置',
    importConfig: '导入配置',
    exportDescription: '当前配置已导出为 JSON 格式，您可以复制到剪贴板或保存为文件。',
    importDescription: '粘贴或加载配置 JSON 来导入设置。导入将覆盖当前配置。',
    importPlaceholder: '在此粘贴配置 JSON...',
    copyToClipboard: '复制到剪贴板',
    saveToFile: '保存为文件',
    loadFromFile: '从文件加载',
    import: '导入',
    importSuccess: '配置导入成功！',

    // Common
    cancel: '取消',
    add: '添加',
    save: '保存',
    delete: '删除',
    ok: '确定',

    // System
    portInUse: '端口 {port} 已被占用。是否终止该端口上的服务并启动代理？',
    confirmDeleteEndpoint: '确定要删除地址 "{name}" 吗？',
    confirmDeleteEndpointFinal: '警告：此操作不可撤销，确定删除吗？',
    deleteLastEndpointError: '必须保留至少一个地址。',
}
