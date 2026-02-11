export default {
    // Status
    statusRunning: 'Proxy Running',
    statusStopped: 'Proxy Stopped',

    // Header
    title: 'Codex Proxy',

    // Config Card
    port: 'Port',
    codexModel: 'Codex Model',
    targetUrl: 'Target URL',
    selectTargetUrl: 'Select target URL',
    edit: 'Edit',
    apiKey: 'Codex API Key',
    apiKeyPlaceholder: 'Optional - Overrides client key',
    apiKeyTip: 'If configured here, you can use any random string as the API key in Claude Code.',
    modelRecommended: 'GPT-5.3-Codex (Recommended)',
    reasoningEffort: 'Reasoning Effort',
    reasoningEffortTip: 'Set default reasoning effort levels for different Claude model families.',
    restoreDefaults: 'Restore Defaults',
    startProxy: 'Start Proxy',
    stopProxy: 'Stop Proxy',

    // Endpoint Dialog
    addEndpoint: 'Add Endpoint',
    editEndpoint: 'Edit Endpoint',
    endpointAlias: 'Alias',
    endpointAliasPlaceholder: 'E.g. Custom Node',
    endpointUrl: 'URL',
    endpointApiKey: 'API Key',

    // Guide
    guideTitle: 'Configuration Guide',
    guideDesc: 'Add the following to your Claude Code settings file:',
    guideTokenHint: 'Replace with real key (or any string if configured in proxy page)',
    copy: 'Copy',
    copied: 'Copied',

    // Logs
    logsTitle: 'System Logs',
    clearLogs: 'Clear Logs',
    noLogs: 'No logs yet...',

    // Menu
    menuPromptSettings: 'Prompt Settings',
    menuConcurrency: 'Concurrency',
    menuAbout: 'About',
    menuLogs: 'Logs',

    // Concurrency
    concurrencyTitle: 'Concurrency Settings',
    concurrencyTip: 'Max concurrent requests. 0 or empty means unlimited. (Optimized for features like Teammate, recommended: 5)',
    concurrencyPlaceholder: '0 = unlimited',

    // About
    aboutTitle: 'About',
    versionLabel: 'Version',
    appName: 'Codex Proxy',
    updateIdle: 'Click "Releases" to check updates',
    updateChecking: 'Checking for updates...',
    updateLatest: 'You are up to date',
    updateAvailable: 'New version available',
    updateFailed: 'Update check failed',
    updateRateLimited: 'Update check failed (GitHub rate limit)',
    goToReleases: 'Releases',

    // Settings
    settingsTitle: 'Settings',
    skillInjection: 'Skill Injection Config',
    skillInjectionTip: 'Inject this prompt into User Message when Skills are used. Leave empty to disable.',
    skillInjectionPlaceholder: 'E.g. Auto-install dependencies if missing...',
    useDefaultPrompt: 'Use Default Prompt',

    // Common
    cancel: 'Cancel',
    add: 'Add',
    save: 'Save',
    ok: 'OK',

    // System
    portInUse: 'Port {port} is in use. Terminate the service and start proxy?',
}
