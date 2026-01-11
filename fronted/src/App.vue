<template>
  <el-container style="height: 100vh;">
    <el-header style="text-align: right; font-size: 12px">
      <div class="toolbar">
        <span>Codex Proxy Controller</span>
      </div>
    </el-header>
    
    <el-main>
      <el-card class="box-card">
        <template #header>
          <div class="card-header">
            <span>Configuration</span>
          </div>
        </template>
        <el-form :model="form" label-width="120px">
          <el-form-item label="Port">
            <el-input v-model.number="form.port" placeholder="8889" />
          </el-form-item>
          <el-form-item label="Target URL">
            <el-input v-model="form.targetUrl" placeholder="https://..." />
          </el-form-item>
          <el-form-item label="API Key">
            <el-input v-model="form.apiKey" type="password" placeholder="Optional" show-password />
          </el-form-item>
          <el-form-item>
            <el-button type="primary" @click="toggleProxy">
              {{ isRunning ? 'Stop Proxy' : 'Start Proxy' }}
            </el-button>
          </el-form-item>
        </el-form>
      </el-card>

      <el-card class="box-card" style="margin-top: 20px;">
        <template #header>
          <div class="card-header">
            <span>Logs</span>
            <el-button style="float: right; padding: 3px 0" text @click="clearLogs">Clear</el-button>
          </div>
        </template>
        <div class="logs-container">
          <div v-for="(log, index) in logs" :key="index" class="log-item">
            {{ log }}
          </div>
        </div>
      </el-card>
    </el-main>
  </el-container>
</template>

<script lang="ts" setup>
import { reactive, ref, onMounted } from 'vue'

const isRunning = ref(false)
const logs = ref<string[]>([])
const form = reactive({
  port: 8889,
  targetUrl: 'https://api.aicodemirror.com/api/codex/backend-api/codex/responses',
  apiKey: ''
})

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
    if (logs.value.length > 1000) logs.value.shift()
  })
})
</script>

<style scoped>
.logs-container {
  height: 300px;
  overflow-y: auto;
  background-color: #f5f7fa;
  padding: 10px;
  border-radius: 4px;
  font-family: monospace;
  font-size: 12px;
}
.log-item {
  margin-bottom: 4px;
  word-break: break-all;
}
</style>
