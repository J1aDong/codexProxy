<template>
  <div class="app-container">
    <div class="main-content">
      <!-- Header -->
      <div class="header-section">
        <div class="status-badge" :class="{ running: isRunning }">
          <div class="status-dot"></div>
          <span class="status-text">{{ isRunning ? 'Running' : 'Stopped' }}</span>
        </div>
        <h1 class="app-title">Codex Proxy</h1>
        <div class="header-actions">
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
              <el-form-item label="Port">
                <el-input v-model.number="form.port" placeholder="8889" />
              </el-form-item>
            </el-col>
            <el-col :span="16">
              <el-form-item label="Target URL">
                <el-input v-model="form.targetUrl" placeholder="https://..." />
              </el-form-item>
            </el-col>
          </el-row>
          
          <el-form-item label="Codex API Key">
            <el-input 
              v-model="form.apiKey" 
              type="password" 
              placeholder="Optional - Overrides client key" 
              show-password 
            />
            <div class="form-tip">
              If configured here, you can use any random string as the API key in Claude Code.
            </div>
          </el-form-item>

          <div class="form-actions">
            <el-button @click="resetDefaults">Restore Defaults</el-button>
            <el-button 
              :type="isRunning ? 'danger' : 'primary'"
              @click="toggleProxy"
              class="primary-btn"
            >
              {{ isRunning ? 'Stop Proxy' : 'Start Proxy' }}
            </el-button>
          </div>
        </el-form>
      </el-card>

      <!-- Guide Section -->
      <div class="guide-section">
        <h3>Configuration Guide</h3>
        <p class="guide-desc">
          Add the following to your Claude Code settings file:<br>
          <code class="path">~/.claude/settings.json</code>
        </p>
        
        <div class="code-block-wrapper">
          <pre class="code-block">{{ configExample }}</pre>
        </div>
      </div>
    </div>

    <!-- Logs Drawer -->
    <el-drawer 
      v-model="showLogs" 
      title="System Logs" 
      direction="rtl" 
      size="400px"
    >
      <div class="logs-container" ref="logsContainer">
        <div v-for="(log, index) in logs" :key="index" class="log-item">
          <span class="log-time">{{ new Date().toLocaleTimeString() }}</span>
          <span class="log-content">{{ log }}</span>
        </div>
      </div>
      <template #footer>
        <div style="flex: auto">
          <el-button @click="clearLogs">Clear</el-button>
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
  // If running, user might want to restart, but we just reset form for now.
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