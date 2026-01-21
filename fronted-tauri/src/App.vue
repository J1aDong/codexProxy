<template>
  <div class="app-container">
    <div class="main-content">
      <!-- Header -->
      <div class="header-section">
        <div class="status-badge" :class="{ running: isRunning }">
          <div class="status-dot"></div>
          <span class="status-text">{{ isRunning ? t.statusRunning : t.statusStopped }}</span>
        </div>
        <h1 class="app-title">{{ t.title }}</h1>
        <div class="header-actions">
           <!-- About -->
           <el-button circle text @click="showAbout = true">
             <el-icon><InfoFilled /></el-icon>
           </el-button>
           <!-- Language Switch -->
           <el-button circle text @click="toggleLang" class="lang-btn">
             {{ lang === 'zh' ? 'En' : '中' }}
           </el-button>
           <!-- Settings -->
           <el-button circle text @click="showSettings = true">
             <el-icon><Setting /></el-icon>
           </el-button>
           <!-- Logs Toggle -->
           <el-button circle text @click="showLogs = true">
             <el-icon><Document /></el-icon>
           </el-button>
        </div>
      </div>

      <!-- Config Card -->
      <el-card class="config-card" shadow="never">
        <el-form :model="form" label-position="top" class="apple-form">
          <el-row :gutter="20">
            <el-col :span="8">
              <el-form-item :label="t.port">
                <el-input v-model.number="form.port" placeholder="8889" />
              </el-form-item>
            </el-col>
            <el-col :span="16">
              <el-form-item :label="t.targetUrl">
                <el-input v-model="form.targetUrl" placeholder="https://..." />
              </el-form-item>
            </el-col>
          </el-row>

          <el-form-item :label="t.apiKey">
            <el-input
              v-model="form.apiKey"
              type="password"
              :placeholder="t.apiKeyPlaceholder"
              show-password
            />
            <div class="form-tip">
              {{ t.apiKeyTip }}
            </div>
          </el-form-item>

          <!-- Reasoning Effort Mapping -->
          <el-divider content-position="left">{{ t.reasoningEffort }}</el-divider>
          <el-row :gutter="16">
            <el-col :span="8">
              <el-form-item label="Opus">
                <el-select v-model="form.reasoningEffort.opus" style="width: 100%">
                  <el-option v-for="opt in effortOptions" :key="opt.value" :label="opt.label" :value="opt.value" />
                </el-select>
              </el-form-item>
            </el-col>
            <el-col :span="8">
              <el-form-item label="Sonnet">
                <el-select v-model="form.reasoningEffort.sonnet" style="width: 100%">
                  <el-option v-for="opt in effortOptions" :key="opt.value" :label="opt.label" :value="opt.value" />
                </el-select>
              </el-form-item>
            </el-col>
            <el-col :span="8">
              <el-form-item label="Haiku">
                <el-select v-model="form.reasoningEffort.haiku" style="width: 100%">
                  <el-option v-for="opt in effortOptions" :key="opt.value" :label="opt.label" :value="opt.value" />
                </el-select>
              </el-form-item>
            </el-col>
          </el-row>
          <div class="form-tip" style="margin-top: -10px; margin-bottom: 20px;">
            {{ t.reasoningEffortTip }}
          </div>

          <div class="form-actions">
            <el-button @click="resetDefaults">{{ t.restoreDefaults }}</el-button>
            <el-button
              :type="isRunning ? 'danger' : 'primary'"
              @click="toggleProxy"
              class="primary-btn"
            >
              {{ isRunning ? t.stopProxy : t.startProxy }}
            </el-button>
          </div>
        </el-form>
      </el-card>

      <!-- Guide Section -->
      <div class="guide-section">
        <h3>{{ t.guideTitle }}</h3>
        <p class="guide-desc">
          {{ t.guideDesc }}<br>
          <code class="path">~/.claude/settings.json</code>
        </p>

        <div class="code-block-wrapper">
          <pre class="code-block">{{ configExample }}</pre>
          <div class="copy-action">
            <el-button size="small" link type="info" @click="copyConfig">
              {{ copied ? t.copied : t.copy }}
            </el-button>
          </div>
        </div>
      </div>
    </div>

    <!-- Logs Drawer -->
    <el-drawer
      v-model="showLogs"
      :title="t.logsTitle"
      direction="rtl"
      size="400px"
    >
      <div class="logs-container" ref="logsContainer">
        <div v-for="(log, index) in logs" :key="index" class="log-item">
          <span class="log-time">{{ new Date().toLocaleTimeString() }}</span>
          <span class="log-content">{{ log }}</span>
        </div>
        <div v-if="logs.length === 0" class="empty-logs">{{ t.noLogs }}</div>
      </div>
      <template #footer>
        <div style="flex: auto">
          <el-button @click="clearLogs">{{ t.clearLogs }}</el-button>
        </div>
      </template>
    </el-drawer>

    <!-- Settings Dialog -->
    <el-dialog v-model="showSettings" :title="t.settingsTitle" width="500px">
      <el-form label-position="top">
        <el-form-item :label="t.skillInjection">
          <el-input
            v-model="form.skillInjectionPrompt"
            type="textarea"
            :rows="4"
            :placeholder="t.skillInjectionPlaceholder"
            maxlength="500"
            show-word-limit
          />
          <div class="form-tip">{{ t.skillInjectionTip }}</div>
          <div style="margin-top: 8px">
            <el-button size="small" link type="primary" @click="useDefaultPrompt">
              {{ t.useDefaultPrompt }}
            </el-button>
          </div>
        </el-form-item>
      </el-form>
      <template #footer>
        <div class="about-footer">
          <el-button type="primary" @click="showSettings = false">OK</el-button>
        </div>
      </template>
    </el-dialog>

    <!-- About Dialog -->
    <el-dialog v-model="showAbout" :title="t.aboutTitle" width="360px">
      <div class="about-body">
        <div class="about-name">{{ t.appName }}</div>
        <div class="about-version">{{ t.versionLabel }} v{{ appVersion }}</div>
        <div class="about-update">
          <div class="update-status">{{ updateStatusText }}</div>
          <el-button size="small" type="primary" plain @click="openReleasePage">
            {{ t.goToReleases }}
          </el-button>
        </div>
      </div>
      <template #footer>
        <div class="about-footer">
          <el-button type="primary" @click="showAbout = false">OK</el-button>
        </div>
      </template>
    </el-dialog>
  </div>
</template>

<script lang="ts" setup>
import { reactive, ref, onMounted, computed, watch, onUnmounted } from 'vue'
import { Document, InfoFilled, Setting } from '@element-plus/icons-vue'
import { ElMessageBox } from 'element-plus'
import { invoke } from '@tauri-apps/api/core'
import { listen, UnlistenFn } from '@tauri-apps/api/event'
import { open } from '@tauri-apps/plugin-shell'
import { fetch } from '@tauri-apps/plugin-http'

const isRunning = ref(false)
const showLogs = ref(false)
const showAbout = ref(false)
const showSettings = ref(false)
const logs = ref<string[]>([])
const logsContainer = ref<HTMLElement | null>(null)
const copied = ref(false)
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
  // Save lang preference immediately
  saveLangPreference()
}

const saveLangPreference = () => {
  invoke('save_lang', { lang: lang.value }).catch(console.error)
}

const translations = {
  zh: {
    statusRunning: '代理运行中',
    statusStopped: '代理已停止',
    title: 'Codex 代理',
    port: '端口',
    targetUrl: '目标地址',
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
    updateIdle: '点击“前往 Release 页面”检查更新',
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
  },
  en: {
    statusRunning: 'Proxy Running',
    statusStopped: 'Proxy Stopped',
    title: 'Codex Proxy',
    port: 'Port',
    targetUrl: 'Target URL',
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
    updateIdle: 'Click “Releases” to check updates',
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
  }
}

const t = computed(() => translations[lang.value])

const updateStatusText = computed(() => {
  if (updateStatus.value === 'checking') return t.value.updateChecking
  if (updateStatus.value === 'failed') {
    if (updateError.value === 'rate_limited') return t.value.updateRateLimited
    return updateError.value
      ? `${t.value.updateFailed} (${updateError.value})`
      : t.value.updateFailed
  }
  if (updateStatus.value === 'available') {
    return `${t.value.updateAvailable} v${latestVersion.value}`
  }
  if (updateStatus.value === 'latest') return t.value.updateLatest
  return t.value.updateIdle
})

const effortOptions = [
  { value: 'low', label: 'Low' },
  { value: 'medium', label: 'Medium' },
  { value: 'high', label: 'High' },
  { value: 'xhigh', label: 'Extra High' },
]

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

watch(showAbout, (visible) => {
  if (!visible) return
  if (updateStatus.value === 'checking') return
  fetchLatestRelease().catch(console.error)
})

const DEFAULT_CONFIG = {
  port: 8889,
  targetUrl: 'https://api.aicodemirror.com/api/codex/backend-api/codex/responses',
  apiKey: '',
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
  reasoningEffort: { ...DEFAULT_CONFIG.reasoningEffort }
})

const useDefaultPrompt = () => {
  form.skillInjectionPrompt = lang.value === 'zh' ? DEFAULT_PROMPT_ZH : DEFAULT_PROMPT_EN
}

const configExample = computed(() => {
  const tokenPlaceholder = lang.value === 'zh'
    ? "替换为真实的key或者假如在proxy页面中配置了则任意字符串"
    : "Replace with real key (or any string if configured in proxy page)";

  return `{
  "env": {
    "ANTHROPIC_BASE_URL": "http://localhost:${form.port}",
    "ANTHROPIC_AUTH_TOKEN": "${tokenPlaceholder}"
  },
  "forceLoginMethod": "claudeai",
  "permissions": {
    "allow": [],
    "deny": []
  }
}`
})

const resetDefaults = () => {
  form.port = DEFAULT_CONFIG.port
  form.targetUrl = DEFAULT_CONFIG.targetUrl
  form.apiKey = DEFAULT_CONFIG.apiKey
  form.reasoningEffort = { ...DEFAULT_CONFIG.reasoningEffort }
  useDefaultPrompt()
}

const toggleProxy = () => {
  if (isRunning.value) {
    invoke('stop_proxy').catch(console.error)
  } else {
    invoke('start_proxy', {
      config: {
        port: form.port,
        targetUrl: form.targetUrl,
        apiKey: form.apiKey,
        reasoningEffort: form.reasoningEffort,
        skillInjectionPrompt: form.skillInjectionPrompt,
        force: false
      }
    }).catch(console.error)
  }
}

const clearLogs = () => {
  logs.value = []
}

const copyConfig = () => {
  navigator.clipboard.writeText(configExample.value)
    .then(() => {
      copied.value = true
      setTimeout(() => { copied.value = false }, 2000)
    })
    .catch(console.error)
}

// Auto scroll logs
watch(logs.value, () => {
  if (showLogs.value && logsContainer.value) {
    setTimeout(() => {
      logsContainer.value!.scrollTop = logsContainer.value!.scrollHeight
    }, 0)
  }
})

onMounted(() => {
  // Load saved config
  invoke<{ port: number; targetUrl: string; apiKey: string; reasoningEffort?: { opus: string; sonnet: string; haiku: string }; skillInjectionPrompt?: string; lang?: string } | null>('load_config')
    .then((savedConfig) => {
      if (savedConfig) {
        if (savedConfig.port) form.port = savedConfig.port
        if (savedConfig.targetUrl) form.targetUrl = savedConfig.targetUrl
        if (savedConfig.apiKey) form.apiKey = savedConfig.apiKey
        if (savedConfig.reasoningEffort) {
          form.reasoningEffort = { ...savedConfig.reasoningEffort }
        }
        if (savedConfig.skillInjectionPrompt !== undefined) {
          form.skillInjectionPrompt = savedConfig.skillInjectionPrompt
        } else {
          // Default for new feature
          useDefaultPrompt()
        }
        if (savedConfig.lang && (savedConfig.lang === 'zh' || savedConfig.lang === 'en')) {
          lang.value = savedConfig.lang
          // If using default prompt (not loaded from config), update it to match loaded lang
          if (savedConfig.skillInjectionPrompt === undefined) {
            useDefaultPrompt()
          }
        }
      } else {
        useDefaultPrompt()
      }
    })
    .catch(console.error)

  // Listen for proxy status
  listen<string>('proxy-status', (event) => {
    isRunning.value = event.payload === 'running'
  }).then(unlisten => unlisteners.push(unlisten))

  // Listen for proxy logs
  listen<string>('proxy-log', (event) => {
    logs.value.push(event.payload)
    if (logs.value.length > 2000) logs.value.shift()
  }).then(unlisten => unlisteners.push(unlisten))

  // Listen for port-in-use
  listen<number>('port-in-use', (event) => {
    const port = event.payload
    ElMessageBox.confirm(
      lang.value === 'zh'
        ? `端口 ${port} 已被占用。是否终止该端口上的服务并启动代理？`
        : `Port ${port} is in use. Do you want to terminate the service on this port and start the proxy?`,
      lang.value === 'zh' ? '端口冲突' : 'Port Conflict',
      {
        confirmButtonText: lang.value === 'zh' ? '终止并启动' : 'Kill & Start',
        cancelButtonText: lang.value === 'zh' ? '取消' : 'Cancel',
        type: 'warning',
      }
    )
      .then(() => {
        invoke('start_proxy', {
          config: {
            port: form.port,
            targetUrl: form.targetUrl,
            apiKey: form.apiKey,
            reasoningEffort: form.reasoningEffort,
            skillInjectionPrompt: form.skillInjectionPrompt,
            force: true
          }
        }).catch(console.error)
      })
      .catch(() => {
        isRunning.value = false
      })
  }).then(unlisten => unlisteners.push(unlisten))
})

onUnmounted(() => {
  unlisteners.forEach(unlisten => unlisten())
})
</script>

<style>
/* Global resets & vars */
:root {
  --bg-color: #f5f5f7;
  --card-bg: #ffffff;
  --text-primary: #1d1d1f;
  --text-secondary: #86868b;
  --accent-color: #0071e3;
  --danger-color: #ff3b30;
  --success-color: #34c759;
  --border-radius: 12px;
}

body {
  margin: 0;
  background-color: var(--bg-color);
  font-family: -apple-system, BlinkMacSystemFont, "SF Pro Text", "Helvetica Neue", Arial, sans-serif;
  color: var(--text-primary);
}

.app-container {
  max-width: 600px;
  margin: 0 auto;
  padding: 40px 20px;
}

.header-section {
  display: flex;
  align-items: center;
  margin-bottom: 30px;
  justify-content: space-between;
}

.app-title {
  font-size: 24px;
  font-weight: 600;
  margin: 0;
  flex-grow: 1;
  text-align: center;
}

.status-badge {
  display: flex;
  align-items: center;
  padding: 6px 12px;
  background: #e5e5e5;
  border-radius: 20px;
  font-size: 12px;
  font-weight: 500;
  color: var(--text-secondary);
  transition: all 0.3s ease;
}

.status-badge.running {
  background: #e1f5e6;
  color: #008a2e;
}

.status-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background-color: currentColor;
  margin-right: 6px;
}

.lang-btn {
  font-weight: 500;
  color: var(--text-secondary);
}

/* Card Styling */
.config-card {
  border: none !important;
  border-radius: var(--border-radius) !important;
  box-shadow: 0 4px 20px rgba(0,0,0,0.04) !important;
  margin-bottom: 30px;
}

.apple-form .el-form-item__label {
  font-weight: 500;
  color: var(--text-primary);
  padding-bottom: 8px;
}

.form-tip {
  font-size: 12px;
  color: var(--text-secondary);
  margin-top: 6px;
  line-height: 1.4;
}

.form-actions {
  display: flex;
  justify-content: space-between;
  margin-top: 30px;
  padding-top: 20px;
  border-top: 1px solid #ebedf0;
}

.primary-btn {
  min-width: 120px;
  font-weight: 500;
}

/* Guide Section */
.guide-section {
  padding: 0 10px;
}

.guide-section h3 {
  font-size: 14px;
  font-weight: 600;
  text-transform: uppercase;
  color: var(--text-secondary);
  letter-spacing: 0.5px;
  margin-bottom: 15px;
}

.guide-desc {
  font-size: 13px;
  color: var(--text-primary);
  margin-bottom: 15px;
  line-height: 1.5;
}

.path {
  background: #eaeaec;
  padding: 2px 6px;
  border-radius: 4px;
  font-family: "SF Mono", monospace;
  font-size: 12px;
}

.code-block-wrapper {
  background: #1e1e1e;
  border-radius: var(--border-radius);
  padding: 15px;
  position: relative;
  overflow: hidden;
}

.code-block {
  margin: 0;
  font-family: "SF Mono", monospace;
  font-size: 12px;
  color: #a9b7c6;
  white-space: pre-wrap;
  line-height: 1.5;
}

.copy-action {
  position: absolute;
  top: 10px;
  right: 10px;
}

/* Logs */
.logs-container {
  height: 100%;
  overflow-y: auto;
  font-family: "SF Mono", monospace;
  font-size: 11px;
  padding: 10px;
}

.log-item {
  margin-bottom: 6px;
  display: flex;
  gap: 8px;
}

.log-time {
  color: var(--text-secondary);
  flex-shrink: 0;
}

.log-content {
  color: var(--text-primary);
  word-break: break-all;
}

.empty-logs {
  text-align: center;
  color: var(--text-secondary);
  margin-top: 40px;
}

.about-body {
  text-align: center;
  padding: 8px 0 4px;
}

.about-name {
  font-size: 16px;
  font-weight: 600;
  margin-bottom: 6px;
}

.about-update {
  margin-top: 10px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.update-status {
  font-size: 12px;
  color: var(--text-secondary);
}

.about-footer {
  display: flex;
  justify-content: flex-end;
}

/* Element Plus overrides */
.el-input__wrapper {
  box-shadow: none !important;
  background-color: #f5f5f7 !important;
  border-radius: 8px !important;
  padding: 8px 12px !important;
}

.el-input__wrapper.is-focus {
  background-color: #ffffff !important;
  box-shadow: 0 0 0 2px var(--accent-color) !important;
}

.el-button {
  border-radius: 8px !important;
  height: 36px !important;
}
</style>
