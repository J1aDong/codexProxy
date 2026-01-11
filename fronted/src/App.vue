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
           <!-- Language Switch -->
           <el-button circle text @click="toggleLang" class="lang-btn">
             {{ lang === 'zh' ? 'En' : '中' }}
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
  </div>
</template>

<script lang="ts" setup>
import { reactive, ref, onMounted, computed, watch } from 'vue'
import { Document } from '@element-plus/icons-vue'

const isRunning = ref(false)
const showLogs = ref(false)
const logs = ref<string[]>([])
const logsContainer = ref<HTMLElement | null>(null)
const copied = ref(false)

// Language Support
const lang = ref<'zh' | 'en'>('zh')
const toggleLang = () => {
  lang.value = lang.value === 'zh' ? 'en' : 'zh'
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
  }
}

const t = computed(() => translations[lang.value])

const DEFAULT_CONFIG = {
  port: 8889,
  targetUrl: 'https://api.aicodemirror.com/api/codex/backend-api/codex/responses',
  apiKey: ''
}

const form = reactive({ ...DEFAULT_CONFIG })

const configExample = computed(() => {
  return `{
  "provider": "@ai-sdk/anthropic",
  "apiKey": "${form.apiKey ? '************' : 'sk-any-string-is-fine'}",
  "anthropic": {
    "baseURL": "http://localhost:${form.port}/messages"
  }
}`
})

const resetDefaults = () => {
  form.port = DEFAULT_CONFIG.port
  form.targetUrl = DEFAULT_CONFIG.targetUrl
  form.apiKey = DEFAULT_CONFIG.apiKey
}

const toggleProxy = () => {
  if (isRunning.value) {
    window.ipcRenderer.send('stop-proxy')
  } else {
    window.ipcRenderer.send('start-proxy', { ...form })
  }
}

const clearLogs = () => {
  logs.value = []
}

const copyConfig = async () => {
  try {
    await navigator.clipboard.writeText(configExample.value)
    copied.value = true
    setTimeout(() => {
      copied.value = false
    }, 2000)
  } catch (err) {
    console.error('Failed to copy', err)
  }
}

// Auto scroll logs
watch(logs.value, () => {
  if (showLogs.value && logsContainer.value) {
    setTimeout(() => {
      logsContainer.value!.scrollTop = logsContainer.value!.scrollHeight
    }, 0)
  }
})

onMounted(async () => {
  const savedConfig = await window.ipcRenderer.invoke('load-config')
  if (savedConfig) {
    if (savedConfig.port) form.port = savedConfig.port
    if (savedConfig.targetUrl) form.targetUrl = savedConfig.targetUrl
    if (savedConfig.apiKey) form.apiKey = savedConfig.apiKey
  }

  window.ipcRenderer.on('proxy-status', (_event, status) => {
    isRunning.value = status === 'running'
  })
  
  window.ipcRenderer.on('proxy-log', (_event, message) => {
    logs.value.push(message)
    if (logs.value.length > 2000) logs.value.shift()
  })
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
