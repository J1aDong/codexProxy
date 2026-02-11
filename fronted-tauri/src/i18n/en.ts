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
    converter: 'Converter',
    converterCodex: 'Codex',
    converterGemini: 'Gemini',
    geminiModel: 'Gemini Model',
    reasoningEffort: 'Reasoning Effort',
    geminiReasoningEffort: 'Gemini Reasoning Effort',
    reasoningEffortTip: 'Set default reasoning effort levels for different Claude model families.',
    geminiReasoningEffortTip: 'Gemini converter uses a single reasoning effort level.',
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
    menuAdvancedSettings: 'Advanced Settings',
    menuAbout: 'About',
    menuLogs: 'Logs',

    // Advanced Settings
    advancedSettingsTitle: 'Advanced Settings',
    advancedMaxConcurrencyLabel: 'Max Concurrent Requests',
    advancedMaxConcurrencyTip: '0 or empty means unlimited. Start with 2-4 and increase gradually.',
    advancedMaxConcurrencyPlaceholder: '0 = unlimited',
    advancedIgnoreProbeLabel: 'Ignore startup probe requests (foo/count)',
    advancedIgnoreProbeTip: 'Return locally for probe-like requests to reduce pointless upstream calls and 429 risk.',
    advancedCountTokensFallbackLabel: 'Allow estimate fallback when count_tokens fails',
    advancedCountTokensFallbackTip: 'Enabled is more stable; disable to surface upstream failures for debugging.',
    advancedSettingsRiskTip: 'Note: ignoring probe requests may affect capability detection for a few clients.',

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
    delete: 'Delete',
    ok: 'OK',

    // System
    portInUse: 'Port {port} is in use. Terminate the service and start proxy?',
    confirmDeleteEndpoint: 'Are you sure you want to delete endpoint "{name}"?',
    confirmDeleteEndpointFinal: 'Warning: This action cannot be undone. Are you sure?',
    deleteLastEndpointError: 'At least one endpoint must be kept.',
}
