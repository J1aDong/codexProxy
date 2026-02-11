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
    reasoningEffort: '推理强度配置',
    reasoningEffortTip: '为不同的 Claude 模型系列设置默认推理强度级别。',
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
    menuConcurrency: '并发数设置',
    menuAbout: '关于',
    menuLogs: '日志',

    // Concurrency
    concurrencyTitle: '并发数设置',
    concurrencyTip: '设置最大并发请求数。0 或留空表示不限制。(优化比如teammate多并发功能一般推荐5)',
    concurrencyPlaceholder: '0 = 不限制',

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

    // Common
    cancel: '取消',
    add: '添加',
    save: '保存',
    ok: '确定',

    // System
    portInUse: '端口 {port} 已被占用。是否终止该端口上的服务并启动代理？',
}
