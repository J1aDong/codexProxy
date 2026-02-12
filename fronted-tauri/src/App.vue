<template>
  <div class="app-container">
    <div class="main-content max-w-3xl mx-auto px-5 py-10">
      <!-- Header -->
      <Header
        :isRunning="isRunning"
        @toggleLang="toggleLang"
        @showAbout="showAbout = true"
        @showSettings="showSettings = true"
        @showAdvancedSettings="openAdvancedSettings"
        @showImportExport="showImportExport = true"
        @showLogs="showLogs = true"
      />

      <!-- Config Card -->
      <ConfigCard
        :form="form"
        :isRunning="isRunning"
        @update:form="updateForm"
        @reset="resetDefaults"
        @toggle="toggleProxy"
        @addEndpoint="openAddEndpointDialog"
        @editEndpoint="handleEditEndpoint"
      />

      <!-- Guide Section -->
      <GuideSection
        :port="form.port"
      />
    </div>

    <!-- Logs Panel -->
    <LogsPanel
      :visible="showLogs"
      :logs="logs"
      :modelRequestStats="modelRequestStats"
      @close="showLogs = false"
      @clear="clearLogs"
    />

    <!-- Endpoint Dialog -->
    <EndpointDialog
      :visible="showEndpointDialog"
      :initial-data="currentEditingEndpoint"
      @close="closeEndpointDialog"
      @add="handleEndpointSubmit"
      @delete="handleDeleteEndpoint"
    />

    <!-- Settings Dialog -->
    <SettingsDialog
      :visible="showSettings"
      :skillInjectionPrompt="form.skillInjectionPrompt"
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
      @close="showAbout = false"
      @checkUpdate="fetchLatestRelease"
      @openReleases="openReleasePage"
    />

    <!-- Import/Export Dialog -->
    <ImportExportDialog
      :visible="showImportExport"
      @close="showImportExport = false"
      @configImported="handleConfigImported"
    />

    <!-- Advanced Settings Dialog -->
    <Dialog
      :visible="showAdvancedSettings"
      :title="t('advancedSettingsTitle')"
      @close="showAdvancedSettings = false"
    >
      <div class="space-y-4">
        <div class="space-y-2">
          <label class="text-sm font-medium text-apple-text-primary">{{ t('advancedMaxConcurrencyLabel') }}</label>
          <input
            type="number"
            v-model.number="localMaxConcurrency"
            min="0"
            max="100"
            class="w-full px-3 py-2.5 rounded-lg bg-gray-100 border border-transparent focus:bg-white focus:border-apple-blue focus:ring-2 focus:ring-apple-blue focus:ring-opacity-20 transition-all duration-200 outline-none"
            :placeholder="t('advancedMaxConcurrencyPlaceholder')"
          />
          <div class="text-apple-text-secondary text-xs">{{ t('advancedMaxConcurrencyTip') }}</div>
        </div>

        <label class="flex items-start gap-3 p-3 rounded-lg border border-gray-200 cursor-pointer">
          <input v-model="localIgnoreProbeRequests" type="checkbox" class="mt-1" />
          <div class="flex-1">
            <div class="text-sm font-medium text-apple-text-primary">{{ t('advancedIgnoreProbeLabel') }}</div>
            <div class="text-xs text-apple-text-secondary mt-1">{{ t('advancedIgnoreProbeTip') }}</div>
          </div>
        </label>

        <label class="flex items-start gap-3 p-3 rounded-lg border border-gray-200 cursor-pointer">
          <input v-model="localAllowCountTokensFallbackEstimate" type="checkbox" class="mt-1" />
          <div class="flex-1">
            <div class="text-sm font-medium text-apple-text-primary">{{ t('advancedCountTokensFallbackLabel') }}</div>
            <div class="text-xs text-apple-text-secondary mt-1">{{ t('advancedCountTokensFallbackTip') }}</div>
          </div>
        </label>

        <div class="space-y-2">
          <div class="flex items-center justify-between">
            <label class="text-sm font-medium text-apple-text-primary">{{ t('advancedCodexCapabilityPresetLabel') }}</label>
            <button
              class="text-xs text-apple-blue hover:text-blue-700 transition-colors"
              @click="resetCodexCapabilityPreset"
            >
              {{ t('restoreDefaults') }}
            </button>
          </div>
          <textarea
            v-model="localCodexCapabilityJson"
            rows="8"
            class="w-full px-3 py-2 rounded-lg bg-gray-100 border border-transparent focus:bg-white focus:border-apple-blue focus:ring-2 focus:ring-apple-blue focus:ring-opacity-20 transition-all duration-200 outline-none font-mono text-xs"
          />
          <div class="text-xs text-apple-text-secondary">{{ t('advancedCodexCapabilityPresetTip') }}</div>
          <div v-if="advancedSettingsError" class="text-xs text-red-500">{{ advancedSettingsError }}</div>
        </div>

        <div class="space-y-2">
          <div class="flex items-center justify-between">
            <label class="text-sm font-medium text-apple-text-primary">{{ t('advancedGeminiModelPresetLabel') }}</label>
            <button
              class="text-xs text-apple-blue hover:text-blue-700 transition-colors"
              @click="resetGeminiModelPreset"
            >
              {{ t('restoreDefaults') }}
            </button>
          </div>
          <textarea
            v-model="localGeminiModelPresetJson"
            rows="5"
            class="w-full px-3 py-2 rounded-lg bg-gray-100 border border-transparent focus:bg-white focus:border-apple-blue focus:ring-2 focus:ring-apple-blue focus:ring-opacity-20 transition-all duration-200 outline-none font-mono text-xs"
          />
          <div class="text-xs text-apple-text-secondary">{{ t('advancedGeminiModelPresetTip') }}</div>
          <div v-if="advancedGeminiPresetError" class="text-xs text-red-500">{{ advancedGeminiPresetError }}</div>
        </div>

        <div class="text-xs text-amber-600 bg-amber-50 border border-amber-200 rounded-lg px-3 py-2">
          {{ t('advancedSettingsRiskTip') }}
        </div>
      </div>

      <template #footer>
        <div class="p-4 flex justify-end">
          <button
            class="px-4 py-2 bg-apple-blue text-white rounded-lg text-sm font-medium hover:bg-blue-600 transition-colors"
            @click="saveAdvancedSettings"
          >
            {{ t('save') }}
          </button>
        </div>
      </template>
    </Dialog>
  </div>
</template>

<script lang="ts" setup>
import { reactive, ref, onMounted, computed, onUnmounted, watch, nextTick } from 'vue'
import { useI18n } from 'vue-i18n'
import { listen, UnlistenFn } from '@tauri-apps/api/event'
import { open } from '@tauri-apps/plugin-shell'
import { fetch } from '@tauri-apps/plugin-http'
import { loadConfig, saveConfig, startProxy, stopProxy, saveLang } from './bridge/configBridge'
import type { CodexEffortCapabilityMap, EndpointOption, GeminiModelPreset, ProxyConfig } from './types/configTypes'

import Header from './components/features/Header.vue'
import ConfigCard from './components/features/ConfigCard.vue'
import GuideSection from './components/features/GuideSection.vue'
import LogsPanel from './components/features/LogsPanel.vue'
import EndpointDialog from './components/features/EndpointDialog.vue'
import SettingsDialog from './components/features/SettingsDialog.vue'
import AboutDialog from './components/features/AboutDialog.vue'
import ImportExportDialog from './components/features/ImportExportDialog.vue'
import Dialog from './components/base/Dialog.vue'

const { t, locale } = useI18n()

const isRunning = ref(false)
const showLogs = ref(false)
const showAbout = ref(false)
const showSettings = ref(false)
const showAdvancedSettings = ref(false)
const showImportExport = ref(false)
const showEndpointDialog = ref(false)

const localMaxConcurrency = ref<number | null>(null)
const localIgnoreProbeRequests = ref(false)
const localAllowCountTokensFallbackEstimate = ref(true)
const localCodexCapabilityJson = ref('')
const localGeminiModelPresetJson = ref('')
const advancedSettingsError = ref('')
const advancedGeminiPresetError = ref('')



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
const lang = computed(() => locale.value as 'zh' | 'en')
const toggleLang = () => {
  locale.value = locale.value === 'zh' ? 'en' : 'zh'
  saveLangPreference()
}

const saveLangPreference = () => {
  saveLang(locale.value).catch(console.error)
}


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
  converter: 'codex' as 'codex' | 'gemini',
  codexModel: 'gpt-5.3-codex',
  codexModelMapping: {
    opus: 'gpt-5.3-codex',
    sonnet: 'gpt-5.2-codex',
    haiku: 'gpt-5.1-codex-mini',
  },
  codexEffortCapabilityMap: {
    'gpt-5.3-codex': ['low', 'medium', 'high', 'xhigh'],
    'gpt-5.2-codex': ['low', 'medium', 'high', 'xhigh'],
    'gpt-5-codex': ['medium', 'high'],
    'gpt-5.1-codex-max': ['low', 'medium', 'high', 'xhigh'],
    'gpt-5.1-codex': ['medium', 'high'],
    'gpt-5.1-codex-mini': ['medium', 'high'],
  } as CodexEffortCapabilityMap,
  geminiModelPreset: [
    'gemini-2.5-flash-lite',
    'gemini-3-pro-preview',
    'gemini-3-pro-image-preview',
    'gemini-3-flash-preview',
    'gemini-2.5-flash',
    'gemini-2.5-pro',
  ] as GeminiModelPreset,
  maxConcurrency: 0,
  ignoreProbeRequests: false,
  allowCountTokensFallbackEstimate: true,
  reasoningEffort: {
    opus: 'xhigh',
    sonnet: 'medium',
    haiku: 'low',
  },
  geminiReasoningEffort: {
    opus: 'gemini-3-pro-preview',
    sonnet: 'gemini-3-flash-preview',
    haiku: 'gemini-3-flash-preview',
  },
  skillInjectionPrompt: '',
}

const DEFAULT_PROMPT_ZH = "skills里的技能如果需要依赖，先安装，不要先用其他方案，如果还有问题告知用户解决方案让用户选择"
const DEFAULT_PROMPT_EN = "If skills require dependencies, install them first. Do not use workarounds. If issues persist, provide solutions for the user to choose."

const form = reactive({
  ...DEFAULT_CONFIG,
  endpointOptions: [...DEFAULT_CONFIG.endpointOptions],
  selectedEndpointId: DEFAULT_CONFIG.selectedEndpointId,
  maxConcurrency: DEFAULT_CONFIG.maxConcurrency,
  ignoreProbeRequests: DEFAULT_CONFIG.ignoreProbeRequests,
  allowCountTokensFallbackEstimate: DEFAULT_CONFIG.allowCountTokensFallbackEstimate,
  reasoningEffort: { ...DEFAULT_CONFIG.reasoningEffort },
  converter: DEFAULT_CONFIG.converter,
  codexModel: DEFAULT_CONFIG.codexModel,
  codexModelMapping: { ...DEFAULT_CONFIG.codexModelMapping },
  codexEffortCapabilityMap: JSON.parse(JSON.stringify(DEFAULT_CONFIG.codexEffortCapabilityMap)),
  geminiModelPreset: [...DEFAULT_CONFIG.geminiModelPreset],
  geminiReasoningEffort: { ...DEFAULT_CONFIG.geminiReasoningEffort },
})

const migrateCodexModel = (model: string): string => {
  return model === 'codex-mini-latest' ? 'gpt-5.1-codex-mini' : model
}

const migrateCodexModelMapping = (mapping: { opus: string; sonnet: string; haiku: string }) => ({
  opus: migrateCodexModel(mapping.opus),
  sonnet: migrateCodexModel(mapping.sonnet),
  haiku: migrateCodexModel(mapping.haiku),
})

const normalizeCapabilityMap = (input: unknown): CodexEffortCapabilityMap => {
  const allowedEfforts = new Set(['low', 'medium', 'high', 'xhigh'])
  const fallback = JSON.parse(JSON.stringify(DEFAULT_CONFIG.codexEffortCapabilityMap)) as CodexEffortCapabilityMap
  if (!input || typeof input !== 'object' || Array.isArray(input)) {
    return fallback
  }

  const result: CodexEffortCapabilityMap = {}
  for (const [model, effortsValue] of Object.entries(input as Record<string, unknown>)) {
    if (!model.trim() || !Array.isArray(effortsValue)) continue
    const normalizedEfforts = effortsValue
      .map((effort) => (typeof effort === 'string' ? effort.toLowerCase() : ''))
      .filter((effort, index, arr) => effort && allowedEfforts.has(effort) && arr.indexOf(effort) === index)

    if (normalizedEfforts.length > 0) {
      result[model] = normalizedEfforts
    }
  }

  if (Object.keys(result).length === 0) {
    return fallback
  }

  return result
}

const normalizeGeminiModelPreset = (input: unknown): GeminiModelPreset => {
  const fallback = [...DEFAULT_CONFIG.geminiModelPreset]
  if (!Array.isArray(input)) return fallback

  const result = input
    .map((item) => (typeof item === 'string' ? item.trim() : ''))
    .filter((item, index, arr) => item.length > 0 && arr.indexOf(item) === index)

  return result.length > 0 ? result : fallback
}

const updateSkillInjectionPrompt = (prompt: string) => {
  form.skillInjectionPrompt = prompt
  saveConfig(buildProxyConfig()).catch(console.error)
}

const openAdvancedSettings = () => {
  localMaxConcurrency.value = form.maxConcurrency
  localIgnoreProbeRequests.value = form.ignoreProbeRequests
  localAllowCountTokensFallbackEstimate.value = form.allowCountTokensFallbackEstimate
  localCodexCapabilityJson.value = JSON.stringify(form.codexEffortCapabilityMap, null, 2)
  localGeminiModelPresetJson.value = JSON.stringify(form.geminiModelPreset, null, 2)
  advancedSettingsError.value = ''
  advancedGeminiPresetError.value = ''
  showAdvancedSettings.value = true
}

const saveAdvancedSettings = () => {
  try {
    const parsed = JSON.parse(localCodexCapabilityJson.value || '{}')
    form.codexEffortCapabilityMap = normalizeCapabilityMap(parsed)
    advancedSettingsError.value = ''
  } catch {
    advancedSettingsError.value = t('advancedCapabilityJsonError')
    return
  }

  try {
    const parsed = JSON.parse(localGeminiModelPresetJson.value || '[]')
    form.geminiModelPreset = normalizeGeminiModelPreset(parsed)
    advancedGeminiPresetError.value = ''
  } catch {
    advancedGeminiPresetError.value = t('advancedGeminiPresetJsonError')
    return
  }

  form.maxConcurrency = localMaxConcurrency.value ?? 0
  form.ignoreProbeRequests = localIgnoreProbeRequests.value
  form.allowCountTokensFallbackEstimate = localAllowCountTokensFallbackEstimate.value
  showAdvancedSettings.value = false
  saveConfig(buildProxyConfig()).catch(console.error)
}

const resetCodexCapabilityPreset = () => {
  localCodexCapabilityJson.value = JSON.stringify(DEFAULT_CONFIG.codexEffortCapabilityMap, null, 2)
}

const resetGeminiModelPreset = () => {
  localGeminiModelPresetJson.value = JSON.stringify(DEFAULT_CONFIG.geminiModelPreset, null, 2)
}

const useDefaultPrompt = () => {
  form.skillInjectionPrompt = lang.value === 'zh' ? DEFAULT_PROMPT_ZH : DEFAULT_PROMPT_EN
}

const handleConfigImported = async () => {
  // 重新加载配置
  const savedConfig = await loadConfig()
  if (savedConfig) {
    if (savedConfig.port) form.port = savedConfig.port
    if (savedConfig.codexModel) {
      form.codexModel = migrateCodexModel(savedConfig.codexModel)
    }
    if (savedConfig.endpointOptions && savedConfig.endpointOptions.length > 0) {
      form.endpointOptions = savedConfig.endpointOptions.map((item) => ({
        ...item,
        codexModel: item.codexModel ? migrateCodexModel(item.codexModel) : item.codexModel,
        codexModelMapping: item.codexModelMapping
          ? migrateCodexModelMapping(item.codexModelMapping)
          : item.codexModelMapping,
      }))
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
    if (savedConfig.skillInjectionPrompt) {
      form.skillInjectionPrompt = savedConfig.skillInjectionPrompt
    } else {
      useDefaultPrompt()
    }
    if (savedConfig.lang && (savedConfig.lang === 'zh' || savedConfig.lang === 'en')) {
      locale.value = savedConfig.lang
    }
    if (typeof savedConfig.maxConcurrency === 'number') {
      form.maxConcurrency = savedConfig.maxConcurrency
    }
    if (savedConfig.codexModelMapping) {
      form.codexModelMapping = migrateCodexModelMapping({
        opus: savedConfig.codexModelMapping.opus || DEFAULT_CONFIG.codexModelMapping.opus,
        sonnet: savedConfig.codexModelMapping.sonnet || DEFAULT_CONFIG.codexModelMapping.sonnet,
        haiku: savedConfig.codexModelMapping.haiku || DEFAULT_CONFIG.codexModelMapping.haiku,
      })
    }
    if (savedConfig.reasoningEffort) {
      form.reasoningEffort = {
        opus: savedConfig.reasoningEffort.opus || DEFAULT_CONFIG.reasoningEffort.opus,
        sonnet: savedConfig.reasoningEffort.sonnet || DEFAULT_CONFIG.reasoningEffort.sonnet,
        haiku: savedConfig.reasoningEffort.haiku || DEFAULT_CONFIG.reasoningEffort.haiku,
      }
    }
    if (savedConfig.geminiReasoningEffort) {
      form.geminiReasoningEffort = {
        opus: savedConfig.geminiReasoningEffort.opus || DEFAULT_CONFIG.geminiReasoningEffort.opus,
        sonnet: savedConfig.geminiReasoningEffort.sonnet || DEFAULT_CONFIG.geminiReasoningEffort.sonnet,
        haiku: savedConfig.geminiReasoningEffort.haiku || DEFAULT_CONFIG.geminiReasoningEffort.haiku,
      }
    }
    if (savedConfig.geminiModelPreset && Array.isArray(savedConfig.geminiModelPreset)) {
      form.geminiModelPreset = savedConfig.geminiModelPreset
    }
    if (typeof savedConfig.ignoreProbeRequests === 'boolean') {
      form.ignoreProbeRequests = savedConfig.ignoreProbeRequests
    }
    if (typeof savedConfig.allowCountTokensFallbackEstimate === 'boolean') {
      form.allowCountTokensFallbackEstimate = savedConfig.allowCountTokensFallbackEstimate
    }
  }
}

const isSyncing = ref(false)

const currentEndpoint = computed(() => {
  const matched = form.endpointOptions.find((item) => item.id === form.selectedEndpointId)
  return matched ?? form.endpointOptions[0] ?? DEFAULT_ENDPOINT_OPTION
})

const syncEndpointFromSelection = () => {
  const endpoint = currentEndpoint.value
  form.targetUrl = endpoint.url
  form.apiKey = endpoint.apiKey
  if (endpoint.converter) form.converter = endpoint.converter
  if (endpoint.codexModel) form.codexModel = migrateCodexModel(endpoint.codexModel)
  if (endpoint.codexModelMapping) form.codexModelMapping = migrateCodexModelMapping(endpoint.codexModelMapping)
  if (endpoint.codexEffortCapabilityMap) {
    form.codexEffortCapabilityMap = normalizeCapabilityMap(endpoint.codexEffortCapabilityMap)
  }
  if (endpoint.geminiModelPreset) {
    form.geminiModelPreset = normalizeGeminiModelPreset(endpoint.geminiModelPreset)
  }
  if (endpoint.reasoningEffort) form.reasoningEffort = { ...endpoint.reasoningEffort }
  if (endpoint.geminiReasoningEffort) form.geminiReasoningEffort = { ...endpoint.geminiReasoningEffort }
}

// Watch for endpoint selection changes to load corresponding config
watch(() => form.selectedEndpointId, async () => {
  isSyncing.value = true
  syncEndpointFromSelection()
  await nextTick()
  isSyncing.value = false
})

const updateSelectedEndpointConfig = () => {
  form.endpointOptions = form.endpointOptions.map((item) => {
    if (item.id !== form.selectedEndpointId) return item
    return {
      ...item,
      url: form.targetUrl,
      apiKey: form.apiKey,
      converter: form.converter,
      codexModel: form.codexModel,
      codexModelMapping: { ...form.codexModelMapping },
      codexEffortCapabilityMap: JSON.parse(JSON.stringify(form.codexEffortCapabilityMap)),
      geminiModelPreset: [...form.geminiModelPreset],
      reasoningEffort: { ...form.reasoningEffort },
      geminiReasoningEffort: { ...form.geminiReasoningEffort },
    }
  })
}

// Watch for changes in current form fields to update back to endpoint options
watch(
  [
    () => form.targetUrl,
    () => form.apiKey,
    () => form.converter,
    () => form.codexModel,
    () => form.codexModelMapping,
    () => form.codexEffortCapabilityMap,
    () => form.geminiModelPreset,
    () => form.reasoningEffort,
    () => form.geminiReasoningEffort,
  ],
  () => {
    if (isSyncing.value) return
    updateSelectedEndpointConfig()
    saveConfig(buildProxyConfig()).catch(console.error)
  },
  { deep: true }
)

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

const handleEndpointSubmit = (endpointData: any) => {
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
      converter: endpointData.converter || form.converter,
      codexModel: endpointData.codexModel || form.codexModel,
      codexModelMapping: endpointData.codexModelMapping || { ...form.codexModelMapping },
      codexEffortCapabilityMap: endpointData.codexEffortCapabilityMap || JSON.parse(JSON.stringify(form.codexEffortCapabilityMap)),
      geminiModelPreset: endpointData.geminiModelPreset || [...form.geminiModelPreset],
      reasoningEffort: endpointData.reasoningEffort || { ...form.reasoningEffort },
      geminiReasoningEffort: endpointData.geminiReasoningEffort || { ...form.geminiReasoningEffort },
    }

    form.endpointOptions = [...form.endpointOptions, nextOption]
    form.selectedEndpointId = nextId
    syncEndpointFromSelection()
  }
  closeEndpointDialog()
}

const handleDeleteEndpoint = (id: string) => {
  if (form.endpointOptions.length <= 1) {
    alert(t('deleteLastEndpointError'))
    return
  }
  
  const index = form.endpointOptions.findIndex(opt => opt.id === id)
  if (index !== -1) {
    form.endpointOptions.splice(index, 1)
    if (form.selectedEndpointId === id) {
      form.selectedEndpointId = form.endpointOptions[0].id
      syncEndpointFromSelection()
    }
    saveConfig(buildProxyConfig()).catch(console.error)
  }
  closeEndpointDialog()
}

const shouldShowLog = (message: string) => {
  if (message.startsWith('[Stat]')) return false
  if (message.includes('[Error]')) return true
  if (message.startsWith('[Req]')) return true
  if (message.startsWith('[ReqPayload]')) return true
  if (message.startsWith('[RateLimit]')) return true
  if (message.startsWith('[Tokens]')) return true
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
  form.converter = DEFAULT_CONFIG.converter
  form.codexModel = DEFAULT_CONFIG.codexModel
  form.codexModelMapping = { ...DEFAULT_CONFIG.codexModelMapping }
  form.codexEffortCapabilityMap = JSON.parse(JSON.stringify(DEFAULT_CONFIG.codexEffortCapabilityMap))
  form.geminiModelPreset = [...DEFAULT_CONFIG.geminiModelPreset]
  form.reasoningEffort = { ...DEFAULT_CONFIG.reasoningEffort }
  form.geminiReasoningEffort = { ...DEFAULT_CONFIG.geminiReasoningEffort }
  form.maxConcurrency = DEFAULT_CONFIG.maxConcurrency
  form.ignoreProbeRequests = DEFAULT_CONFIG.ignoreProbeRequests
  form.allowCountTokensFallbackEstimate = DEFAULT_CONFIG.allowCountTokensFallbackEstimate
  useDefaultPrompt()
}

const buildProxyConfig = (force = false): ProxyConfig => ({
  port: form.port,
  targetUrl: form.targetUrl,
  apiKey: form.apiKey,
  endpointOptions: form.endpointOptions,
  selectedEndpointId: form.selectedEndpointId,
  converter: form.converter,
  codexModel: form.codexModel,
  codexModelMapping: form.codexModelMapping,
  codexEffortCapabilityMap: form.codexEffortCapabilityMap,
  geminiModelPreset: form.geminiModelPreset,
  maxConcurrency: form.maxConcurrency,
  ignoreProbeRequests: form.ignoreProbeRequests,
  allowCountTokensFallbackEstimate: form.allowCountTokensFallbackEstimate,
  reasoningEffort: form.reasoningEffort,
  geminiReasoningEffort: form.geminiReasoningEffort,
  skillInjectionPrompt: form.skillInjectionPrompt,
  lang: locale.value,
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
        if (savedConfig.codexModel) {
          form.codexModel = migrateCodexModel(savedConfig.codexModel)
        }
        if (savedConfig.endpointOptions && savedConfig.endpointOptions.length > 0) {
          form.endpointOptions = savedConfig.endpointOptions.map((item) => ({
            ...item,
            codexModel: item.codexModel ? migrateCodexModel(item.codexModel) : item.codexModel,
            codexModelMapping: item.codexModelMapping
              ? migrateCodexModelMapping(item.codexModelMapping)
              : item.codexModelMapping,
          }))
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
        if (savedConfig.skillInjectionPrompt) {
          form.skillInjectionPrompt = savedConfig.skillInjectionPrompt
        } else {
          useDefaultPrompt()
        }
        if (savedConfig.lang && (savedConfig.lang === 'zh' || savedConfig.lang === 'en')) {
          locale.value = savedConfig.lang
        }
        if (typeof savedConfig.maxConcurrency === 'number') {
          form.maxConcurrency = savedConfig.maxConcurrency
        }
        if (savedConfig.codexModelMapping) {
          form.codexModelMapping = migrateCodexModelMapping({
            opus: savedConfig.codexModelMapping.opus || DEFAULT_CONFIG.codexModelMapping.opus,
            sonnet: savedConfig.codexModelMapping.sonnet || DEFAULT_CONFIG.codexModelMapping.sonnet,
            haiku: savedConfig.codexModelMapping.haiku || DEFAULT_CONFIG.codexModelMapping.haiku,
          })
        }
        if (savedConfig.codexEffortCapabilityMap) {
          form.codexEffortCapabilityMap = normalizeCapabilityMap(savedConfig.codexEffortCapabilityMap)
        }
        if (savedConfig.geminiModelPreset) {
          form.geminiModelPreset = normalizeGeminiModelPreset(savedConfig.geminiModelPreset)
        }
        if (typeof savedConfig.ignoreProbeRequests === 'boolean') {
          form.ignoreProbeRequests = savedConfig.ignoreProbeRequests
        }
        if (typeof savedConfig.allowCountTokensFallbackEstimate === 'boolean') {
          form.allowCountTokensFallbackEstimate = savedConfig.allowCountTokensFallbackEstimate
        }
        localMaxConcurrency.value = form.maxConcurrency
        localIgnoreProbeRequests.value = form.ignoreProbeRequests
        localAllowCountTokensFallbackEstimate.value = form.allowCountTokensFallbackEstimate
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
      t('portInUse', { port })
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
})

watch(
  [
    () => form.maxConcurrency,
    () => form.ignoreProbeRequests,
    () => form.allowCountTokensFallbackEstimate,
  ],
  () => {
    if (!isSyncing.value) {
      saveConfig(buildProxyConfig()).catch(console.error)
    }
  }
)
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
