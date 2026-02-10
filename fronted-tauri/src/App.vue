<template>
  <div class="app-container">
    <div class="main-content max-w-3xl mx-auto px-5 py-10">
      <!-- Header -->
      <Header
        :isRunning="isRunning"
        :lang="lang"
        :t="t"
        @toggleLang="toggleLang"
        @showAbout="showAbout = true"
        @showSettings="showSettings = true"
        @showLogs="showLogs = true"
      />

      <!-- Config Card -->
      <ConfigCard
        :form="form"
        :isRunning="isRunning"
        :t="t"
        @update:form="updateForm"
        @reset="resetDefaults"
        @toggle="toggleProxy"
        @addEndpoint="openAddEndpointDialog"
        @editEndpoint="handleEditEndpoint"
      />

      <!-- Guide Section -->
      <GuideSection
        :port="form.port"
        :lang="lang"
        :t="t"
      />
    </div>

    <!-- Logs Panel -->
    <LogsPanel
      :visible="showLogs"
      :logs="logs"
      :modelRequestStats="modelRequestStats"
      :t="t"
      @close="showLogs = false"
      @clear="clearLogs"
    />

    <!-- Endpoint Dialog -->
    <EndpointDialog
      :visible="showEndpointDialog"
      :initial-data="currentEditingEndpoint"
      :t="dialogT"
      @close="closeEndpointDialog"
      @add="handleEndpointSubmit"
    />

    <!-- Settings Dialog -->
    <SettingsDialog
      :visible="showSettings"
      :skillInjectionPrompt="form.skillInjectionPrompt"
      :lang="lang"
      :t="t"
      @close="showSettings = false"
      @update="updateSkillInjectionPrompt"
    />

    <!-- About Dialog -->
    <AboutDialog
      :visible="showAbout"
      :appVersion="appVersion"
      :updateStatus="updateStatus"
      :latestVersion="latestVersion"
      :updateError="updateError"
      :t="t"
      @close="showAbout = false"
      @checkUpdate="fetchLatestRelease"
      @openReleases="openReleasePage"
    />
  </div>
</template>

<script lang="ts" setup>
import { reactive, ref, onMounted, computed, onUnmounted, watch } from 'vue'
import { listen, UnlistenFn } from '@tauri-apps/api/event'
import { open } from '@tauri-apps/plugin-shell'
import { fetch } from '@tauri-apps/plugin-http'
import { loadConfig, saveConfig, startProxy, stopProxy, saveLang } from './bridge/configBridge'
import type { EndpointOption, ProxyConfig } from './types/configTypes'

import Header from './components/features/Header.vue'
import ConfigCard from './components/features/ConfigCard.vue'
import GuideSection from './components/features/GuideSection.vue'
import LogsPanel from './components/features/LogsPanel.vue'
import EndpointDialog from './components/features/EndpointDialog.vue'
import SettingsDialog from './components/features/SettingsDialog.vue'
import AboutDialog from './components/features/AboutDialog.vue'

const isRunning = ref(false)
const showLogs = ref(false)
const showAbout = ref(false)
const showSettings = ref(false)
const showEndpointDialog = ref(false)



type LogItem = {
  time: string
  content: string
}

const logs = ref<LogItem[]>([])
const modelRequestStats = reactive({
  opus: 0,
  sonnet: 0,
  haiku: 0,
})

const appVersion = __APP_VERSION__
const updateStatus = ref<'idle' | 'checking' | 'latest' | 'available' | 'failed'>('idle')
const latestVersion = ref('')
const updateError = ref('')
const updateRequestId = ref(0)

const RELEASES_URL = 'https://github.com/J1aDong/codexProxy/releases'

// Event listeners
const unlisteners: UnlistenFn[] = []

// Language Support
const lang = ref<'zh' | 'en'>('zh')
const toggleLang = () => {
  lang.value = lang.value === 'zh' ? 'en' : 'zh'
  saveLangPreference()
}

const saveLangPreference = () => {
  saveLang(lang.value).catch(console.error)
}

const translations = {
  zh: {
    statusRunning: '代理运行中',
    statusStopped: '代理已停止',
    title: 'Codex 代理',
    port: '端口',
    codexModel: 'Codex 模型',
    targetUrl: '目标地址',
    addEndpoint: '添加地址',
    editEndpoint: '编辑地址',
    endpointAlias: '别名',
    endpointAliasPlaceholder: '例如：自建节点',
    endpointUrl: '地址',
    endpointApiKey: '密钥',
    apiKey: 'Codex API 密钥',
    apiKeyPlaceholder: '选填 - 将覆盖客户端提供的密钥',
    apiKeyTip: '如果在此处配置，您可以在 Claude Code 中使用任意随机字符串作为 API 密钥。',
    restoreDefaults: '恢复默认',
    startProxy: '启动代理',
    stopProxy: '停止代理',
    guideTitle: '配置指南',
    guideDesc: '请将以下内容添加到您的 Claude Code 配置文件：',
    logsTitle: '系统日志',
    clearLogs: '清除日志',
    copy: '复制',
    copied: '已复制',
    noLogs: '暂无日志...',
    reasoningEffort: '推理强度配置',
    reasoningEffortTip: '为不同的 Claude 模型系列设置默认推理强度级别。',
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
    settingsTitle: '设置',
    skillInjection: '技能注入配置',
    skillInjectionTip: '当使用 Skill 时，将此提示词注入到 User Message 中。留空则不注入。',
    skillInjectionPlaceholder: '例如：如果依赖缺失，请自动安装...',
    useDefaultPrompt: '使用默认提示词',
    cancel: '取消',
    add: '添加',
    save: '保存',
  },
  en: {
    statusRunning: 'Proxy Running',
    statusStopped: 'Proxy Stopped',
    title: 'Codex Proxy',
    port: 'Port',
    codexModel: 'Codex Model',
    targetUrl: 'Target URL',
    addEndpoint: 'Add Endpoint',
    editEndpoint: 'Edit Endpoint',
    endpointAlias: 'Alias',
    endpointAliasPlaceholder: 'E.g. Custom Node',
    endpointUrl: 'URL',
    endpointApiKey: 'API Key',
    apiKey: 'Codex API Key',
    apiKeyPlaceholder: 'Optional - Overrides client key',
    apiKeyTip: 'If configured here, you can use any random string as the API key in Claude Code.',
    restoreDefaults: 'Restore Defaults',
    startProxy: 'Start Proxy',
    stopProxy: 'Stop Proxy',
    guideTitle: 'Configuration Guide',
    guideDesc: 'Add the following to your Claude Code settings file:',
    logsTitle: 'System Logs',
    clearLogs: 'Clear Logs',
    copy: 'Copy',
    copied: 'Copied',
    noLogs: 'No logs yet...',
    reasoningEffort: 'Reasoning Effort',
    reasoningEffortTip: 'Set default reasoning effort levels for different Claude model families.',
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
    settingsTitle: 'Settings',
    skillInjection: 'Skill Injection Config',
    skillInjectionTip: 'Inject this prompt into User Message when Skills are used. Leave empty to disable.',
    skillInjectionPlaceholder: 'E.g. Auto-install dependencies if missing...',
    useDefaultPrompt: 'Use Default Prompt',
    cancel: 'Cancel',
    add: 'Add',
    save: 'Save',
  }
}

const t = computed(() => translations[lang.value])

const dialogT = computed(() => ({
  addEndpoint: t.value.addEndpoint,
  endpointAlias: t.value.endpointAlias,
  endpointAliasPlaceholder: t.value.endpointAliasPlaceholder,
  endpointUrl: t.value.endpointUrl,
  endpointApiKey: t.value.endpointApiKey,
  apiKeyPlaceholder: t.value.apiKeyPlaceholder,
  cancel: t.value.cancel,
  add: t.value.add,
  save: t.value.save,
  editEndpoint: t.value.editEndpoint,
}))

const DEFAULT_ENDPOINT_OPTION: EndpointOption = {
  id: 'aicodemirror-default',
  alias: 'aicodemirror',
  url: 'https://api.aicodemirror.com/api/codex/backend-api/codex/responses',
  apiKey: '',
}

const DEFAULT_CONFIG = {
  port: 8889,
  targetUrl: DEFAULT_ENDPOINT_OPTION.url,
  apiKey: '',
  endpointOptions: [DEFAULT_ENDPOINT_OPTION],
  selectedEndpointId: DEFAULT_ENDPOINT_OPTION.id,
  codexModel: 'gpt-5.3-codex',
  reasoningEffort: {
    opus: 'xhigh',
    sonnet: 'medium',
    haiku: 'low',
  },
  skillInjectionPrompt: '',
}

const DEFAULT_PROMPT_ZH = "skills里的技能如果需要依赖，先安装，不要先用其他方案，如果还有问题告知用户解决方案让用户选择"
const DEFAULT_PROMPT_EN = "If skills require dependencies, install them first. Do not use workarounds. If issues persist, provide solutions for the user to choose."

const form = reactive({
  ...DEFAULT_CONFIG,
  endpointOptions: [...DEFAULT_CONFIG.endpointOptions],
  selectedEndpointId: DEFAULT_CONFIG.selectedEndpointId,
  reasoningEffort: { ...DEFAULT_CONFIG.reasoningEffort }
})

const updateSkillInjectionPrompt = (prompt: string) => {
  form.skillInjectionPrompt = prompt
  saveConfig(buildProxyConfig()).catch(console.error)
}

const useDefaultPrompt = () => {
  form.skillInjectionPrompt = lang.value === 'zh' ? DEFAULT_PROMPT_ZH : DEFAULT_PROMPT_EN
}

const currentEndpoint = computed(() => {
  const matched = form.endpointOptions.find((item) => item.id === form.selectedEndpointId)
  return matched ?? form.endpointOptions[0] ?? DEFAULT_ENDPOINT_OPTION
})

const syncEndpointFromSelection = () => {
  form.targetUrl = currentEndpoint.value.url
  form.apiKey = currentEndpoint.value.apiKey
}

const updateSelectedEndpointApiKey = (nextApiKey: string) => {
  form.endpointOptions = form.endpointOptions.map((item) => {
    if (item.id !== form.selectedEndpointId) return item
    if (item.apiKey === nextApiKey) return item
    return {
      ...item,
      apiKey: nextApiKey,
    }
  })
}

// Watch for API key changes to update endpoint options
const unwatchApiKey = watch(() => form.apiKey, (newValue: string) => {
  updateSelectedEndpointApiKey(newValue)
})

const updateForm = (newForm: any) => {
  Object.assign(form, newForm)
}

const editingEndpointId = ref('')

const currentEditingEndpoint = computed(() => {
  if (!editingEndpointId.value) return undefined
  return form.endpointOptions.find(opt => opt.id === editingEndpointId.value)
})

const openAddEndpointDialog = () => {
  editingEndpointId.value = ''
  showEndpointDialog.value = true
}

const handleEditEndpoint = (id: string) => {
  editingEndpointId.value = id
  showEndpointDialog.value = true
}

const closeEndpointDialog = () => {
  showEndpointDialog.value = false
  editingEndpointId.value = ''
}

const handleEndpointSubmit = (endpointData: { alias: string; url: string; apiKey: string }) => {
  if (editingEndpointId.value) {
    // Edit mode
    const index = form.endpointOptions.findIndex(opt => opt.id === editingEndpointId.value)
    if (index !== -1) {
      form.endpointOptions[index] = {
        ...form.endpointOptions[index],
        ...endpointData
      }
      if (form.selectedEndpointId === editingEndpointId.value) {
        syncEndpointFromSelection()
      }
    }
  } else {
    // Add mode
    const nextId = `endpoint-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`
    const nextOption: EndpointOption = {
      id: nextId,
      ...endpointData,
    }

    form.endpointOptions = [...form.endpointOptions, nextOption]
    form.selectedEndpointId = nextId
    syncEndpointFromSelection()
  }
  closeEndpointDialog()
}

const shouldShowLog = (message: string) => {
  if (message.startsWith('[Stat]')) return false
  if (message.includes('[Error]')) return true
  if (message.includes('[System] Init success')) return true
  if (message.includes('[Request] Sending request')) return true
  return false
}

const pushLog = (message: string) => {
  if (!shouldShowLog(message)) return
  const nextLog: LogItem = {
    time: new Date().toLocaleTimeString(),
    content: message,
  }
  logs.value = [...logs.value, nextLog].slice(-20)
}

const tryCountModelStat = (message: string) => {
  if (!message.startsWith('[Stat] model_request:')) return
  const family = message.replace('[Stat] model_request:', '').trim()
  if (family === 'opus') modelRequestStats.opus += 1
  if (family === 'sonnet') modelRequestStats.sonnet += 1
  if (family === 'haiku') modelRequestStats.haiku += 1
}

const resetDefaults = () => {
  form.port = DEFAULT_CONFIG.port
  form.endpointOptions = [...DEFAULT_CONFIG.endpointOptions]
  form.selectedEndpointId = DEFAULT_CONFIG.selectedEndpointId
  syncEndpointFromSelection()
  form.codexModel = DEFAULT_CONFIG.codexModel
  form.reasoningEffort = { ...DEFAULT_CONFIG.reasoningEffort }
  useDefaultPrompt()
}

const buildProxyConfig = (force = false): ProxyConfig => ({
  port: form.port,
  targetUrl: form.targetUrl,
  apiKey: form.apiKey,
  endpointOptions: form.endpointOptions,
  selectedEndpointId: form.selectedEndpointId,
  codexModel: form.codexModel,
  reasoningEffort: form.reasoningEffort,
  skillInjectionPrompt: form.skillInjectionPrompt,
  lang: lang.value,
  force,
})

const toggleProxy = () => {
  if (isRunning.value) {
    stopProxy().catch(console.error)
  } else {
    startProxy(buildProxyConfig(false)).catch(console.error)
  }
}

const clearLogs = () => {
  logs.value = []
}

// Version checking
const parseVersionParts = (version: string) => {
  const cleaned = version.trim().replace(/^v/i, '')
  const parts = cleaned.split('.')
  const normalized = [0, 0, 0]
  for (let i = 0; i < 3; i += 1) {
    const value = Number(parts[i])
    normalized[i] = Number.isFinite(value) ? value : 0
  }
  return normalized
}

const compareSemver = (current: string, latest: string) => {
  const currentParts = parseVersionParts(current)
  const latestParts = parseVersionParts(latest)
  for (let i = 0; i < 3; i += 1) {
    if (latestParts[i] > currentParts[i]) return 1
    if (latestParts[i] < currentParts[i]) return -1
  }
  return 0
}

const extractTagName = (value: string) => {
  const match = value.match(/\/tag\/(v?\d+\.\d+\.\d+)/i)
  return match ? match[1] : ''
}

const fetchLatestReleaseFromWeb = () => {
  const url = 'https://github.com/J1aDong/codexProxy/releases/latest'
  return fetch(url, {
    method: 'GET',
    redirect: 'follow'
  })
    .then((response) => {
      if (!response.ok) {
        throw new Error(`status ${response.status}`)
      }
      const tagFromUrl = extractTagName(response.url)
      if (tagFromUrl) return tagFromUrl
      return response.text().then((html) => {
        const tagFromHtml = extractTagName(html)
        if (!tagFromHtml) {
          throw new Error('missing tag')
        }
        return tagFromHtml
      })
    })
}

const fetchLatestRelease = () => {
  updateStatus.value = 'checking'
  updateError.value = ''
  updateRequestId.value += 1
  const requestId = updateRequestId.value
  const apiUrl = 'https://api.github.com/repos/J1aDong/codexProxy/releases/latest'
  return fetch(apiUrl, {
    method: 'GET',
    headers: {
      Accept: 'application/vnd.github+json',
      'X-GitHub-Api-Version': '2022-11-28',
      'User-Agent': 'codex-proxy-tauri'
    }
  })
    .then((response) => {
      if (!response.ok) {
        if (response.status === 403) {
          const remaining = response.headers.get('x-ratelimit-remaining')
          if (remaining === '0') {
            throw new Error('rate_limited')
          }
        }
        throw new Error(`status ${response.status}`)
      }
      return response.json()
    })
    .then((data) => {
      const tagName = typeof data?.tag_name === 'string' ? data.tag_name : ''
      if (!tagName) {
        throw new Error('missing tag')
      }
      return tagName
    })
    .catch((error) => {
      if (error instanceof Error && error.message === 'rate_limited') {
        return fetchLatestReleaseFromWeb()
      }
      throw error
    })
    .then((tagName) => {
      if (requestId !== updateRequestId.value) return
      latestVersion.value = tagName.replace(/^v/i, '')
      updateStatus.value = compareSemver(appVersion, latestVersion.value) === 1
        ? 'available'
        : 'latest'
    })
    .catch((error) => {
      if (requestId !== updateRequestId.value) return
      updateStatus.value = 'failed'
      updateError.value = typeof error === 'string'
        ? error
        : error instanceof Error
          ? error.message
          : JSON.stringify(error)
    })
}

const openReleasePage = () => {
  open(RELEASES_URL).catch(console.error)
}

onMounted(() => {
  loadConfig()
    .then((savedConfig) => {
      if (savedConfig) {
        if (savedConfig.port) form.port = savedConfig.port
        if (savedConfig.endpointOptions && savedConfig.endpointOptions.length > 0) {
          form.endpointOptions = [...savedConfig.endpointOptions]
          if (savedConfig.selectedEndpointId) {
            form.selectedEndpointId = savedConfig.selectedEndpointId
          }
          const hasSelected = form.endpointOptions.some((item) => item.id === form.selectedEndpointId)
          if (!hasSelected) {
            form.selectedEndpointId = form.endpointOptions[0].id
          }
        } else {
          const legacyOption: EndpointOption = {
            id: DEFAULT_ENDPOINT_OPTION.id,
            alias: 'aicodemirror',
            url: savedConfig.targetUrl || DEFAULT_ENDPOINT_OPTION.url,
            apiKey: savedConfig.apiKey || '',
          }
          form.endpointOptions = [legacyOption]
          form.selectedEndpointId = legacyOption.id
        }
        syncEndpointFromSelection()
        if (savedConfig.codexModel) form.codexModel = savedConfig.codexModel
        if (savedConfig.reasoningEffort) {
          form.reasoningEffort = { ...savedConfig.reasoningEffort }
        }
        if (savedConfig.skillInjectionPrompt) {
          form.skillInjectionPrompt = savedConfig.skillInjectionPrompt
        } else {
          useDefaultPrompt()
        }
        if (savedConfig.lang && (savedConfig.lang === 'zh' || savedConfig.lang === 'en')) {
          lang.value = savedConfig.lang
          if (!savedConfig.skillInjectionPrompt) {
            useDefaultPrompt()
          }
        }
      } else {
        syncEndpointFromSelection()
        useDefaultPrompt()
        saveConfig(buildProxyConfig()).catch(console.error)
      }
    })
    .catch(console.error)

  // Listen for proxy status
  listen<string>('proxy-status', (event) => {
    isRunning.value = event.payload === 'running'
  }).then(unlisten => unlisteners.push(unlisten))

  // Listen for proxy logs
  listen<string>('proxy-log', (event) => {
    const message = event.payload
    tryCountModelStat(message)
    pushLog(message)
  }).then(unlisten => unlisteners.push(unlisten))

  // Listen for port-in-use
  listen<number>('port-in-use', (event) => {
    const port = event.payload
    const confirmed = confirm(
      lang.value === 'zh'
        ? `端口 ${port} 已被占用。是否终止该端口上的服务并启动代理？`
        : `Port ${port} is in use. Do you want to terminate the service on this port and start the proxy?`
    )
    if (confirmed) {
      startProxy(buildProxyConfig(true)).catch(console.error)
    } else {
      isRunning.value = false
    }
  }).then(unlisten => unlisteners.push(unlisten))
})

onUnmounted(() => {
  unlisteners.forEach(unlisten => unlisten())
  unwatchApiKey()
})
</script>

<style scoped>
.app-container {
  min-height: 100vh;
  background-color: #f5f5f7;
  font-family: -apple-system, BlinkMacSystemFont, "SF Pro Text", "Helvetica Neue", Arial, sans-serif;
}

.main-content {
  max-width: 600px;
  margin: 0 auto;
  padding: 40px 20px;
}
</style>
