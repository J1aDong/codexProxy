<template>
  <div class="app-container" :class="{ 'dark': isDarkMode }">
    <div class="main-content max-w-3xl mx-auto px-5 py-10">
      <!-- Header -->
      <Header
        :isRunning="isRunning"
        :isDarkMode="isDarkMode"
        @toggleLang="toggleLang"
        @toggleDarkMode="toggleDarkMode"
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
        :lbAvailabilityMap="lbAvailabilityMap"
        :isDarkMode="isDarkMode"
        @update:form="updateForm"
        @reset="resetDefaults"
        @toggle="toggleProxy"
        @addEndpoint="openAddEndpointDialog"
        @editEndpoint="handleEditEndpoint"
      />

      <!-- Guide Section -->
      <GuideSection
        :port="form.port"
        :isDarkMode="isDarkMode"
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
          <label class="text-sm font-medium text-apple-text-primary dark:text-dark-text-primary">{{ t('advancedMaxConcurrencyLabel') }}</label>
          <input
            type="number"
            v-model.number="localMaxConcurrency"
            min="0"
            max="100"
            class="w-full px-3 py-2.5 rounded-lg bg-gray-100 dark:bg-dark-tertiary dark:border-dark-border dark:text-dark-text-primary border border-transparent focus:bg-white dark:focus:bg-dark-tertiary focus:border-apple-blue dark:focus:border-accent-blue focus:ring-2 focus:ring-apple-blue focus:ring-opacity-20 transition-all duration-200 outline-none"
            :placeholder="t('advancedMaxConcurrencyPlaceholder')"
          />
          <div class="text-apple-text-secondary dark:text-dark-text-secondary text-xs">{{ t('advancedMaxConcurrencyTip') }}</div>
        </div>

        <div class="space-y-2">
          <label class="text-sm font-medium text-apple-text-primary dark:text-dark-text-primary">{{ t('advancedLbModelCooldownLabel') }}</label>
          <input
            type="number"
            v-model.number="localLbModelCooldownSeconds"
            min="1"
            max="86400"
            class="w-full px-3 py-2.5 rounded-lg bg-gray-100 dark:bg-dark-tertiary dark:border-dark-border dark:text-dark-text-primary border border-transparent focus:bg-white dark:focus:bg-dark-tertiary focus:border-apple-blue dark:focus:border-accent-blue focus:ring-2 focus:ring-apple-blue focus:ring-opacity-20 transition-all duration-200 outline-none"
            :placeholder="t('advancedLbModelCooldownPlaceholder')"
          />
          <div class="text-apple-text-secondary dark:text-dark-text-secondary text-xs">{{ t('advancedLbModelCooldownTip') }}</div>
        </div>

        <div class="space-y-2">
          <label class="text-sm font-medium text-apple-text-primary dark:text-dark-text-primary">{{ t('advancedLbTransientBackoffLabel') }}</label>
          <input
            type="number"
            v-model.number="localLbTransientBackoffSeconds"
            min="1"
            max="120"
            class="w-full px-3 py-2.5 rounded-lg bg-gray-100 dark:bg-dark-tertiary dark:border-dark-border dark:text-dark-text-primary border border-transparent focus:bg-white dark:focus:bg-dark-tertiary focus:border-apple-blue dark:focus:border-accent-blue focus:ring-2 focus:ring-apple-blue focus:ring-opacity-20 transition-all duration-200 outline-none"
            :placeholder="t('advancedLbTransientBackoffPlaceholder')"
          />
          <div class="text-apple-text-secondary dark:text-dark-text-secondary text-xs">{{ t('advancedLbTransientBackoffTip') }}</div>
        </div>

        <label
          class="flex items-start gap-3 p-3 rounded-lg border cursor-pointer dark:border-dark-border"
          :class="form.proxyMode === 'load_balancer' ? 'border-dark-border' : 'border-gray-200'"
        >
          <input v-model="localIgnoreProbeRequests" type="checkbox" class="mt-1" />
          <div class="flex-1">
            <div class="text-sm font-medium text-apple-text-primary dark:text-dark-text-primary">{{ t('advancedIgnoreProbeLabel') }}</div>
            <div class="text-xs text-apple-text-secondary dark:text-dark-text-secondary mt-1">{{ t('advancedIgnoreProbeTip') }}</div>
          </div>
        </label>

        <label
          class="flex items-start gap-3 p-3 rounded-lg border cursor-pointer dark:border-dark-border"
          :class="form.proxyMode === 'load_balancer' ? 'border-dark-border' : 'border-gray-200'"
        >
          <input v-model="localAllowCountTokensFallbackEstimate" type="checkbox" class="mt-1" />
          <div class="flex-1">
            <div class="text-sm font-medium text-apple-text-primary dark:text-dark-text-primary">{{ t('advancedCountTokensFallbackLabel') }}</div>
            <div class="text-xs text-apple-text-secondary dark:text-dark-text-secondary mt-1">{{ t('advancedCountTokensFallbackTip') }}</div>
          </div>
        </label>

        <div class="space-y-2">
          <div class="flex items-center justify-between">
            <label class="text-sm font-medium text-apple-text-primary dark:text-dark-text-primary">{{ t('advancedCodexCapabilityPresetLabel') }}</label>
            <button
              class="text-xs text-apple-blue dark:text-accent-blue hover:text-blue-700 dark:hover:text-blue-400 transition-colors"
              @click="resetCodexCapabilityPreset"
            >
              {{ t('restoreDefaults') }}
            </button>
          </div>
          <textarea
            v-model="localCodexCapabilityJson"
            rows="8"
            class="w-full px-3 py-2 rounded-lg bg-gray-100 dark:bg-dark-tertiary dark:border-dark-border dark:text-dark-text-primary border border-transparent focus:bg-white dark:focus:bg-dark-tertiary focus:border-apple-blue dark:focus:border-accent-blue focus:ring-2 focus:ring-apple-blue focus:ring-opacity-20 transition-all duration-200 outline-none font-mono text-xs"
          />
          <div class="text-xs text-apple-text-secondary dark:text-dark-text-secondary">{{ t('advancedCodexCapabilityPresetTip') }}</div>
          <div v-if="advancedSettingsError" class="text-xs text-red-500">{{ advancedSettingsError }}</div>
        </div>

        <div class="space-y-2">
          <div class="flex items-center justify-between">
            <label class="text-sm font-medium text-apple-text-primary dark:text-dark-text-primary">{{ t('advancedGeminiModelPresetLabel') }}</label>
            <button
              class="text-xs text-apple-blue dark:text-accent-blue hover:text-blue-700 dark:hover:text-blue-400 transition-colors"
              @click="resetGeminiModelPreset"
            >
              {{ t('restoreDefaults') }}
            </button>
          </div>
          <textarea
            v-model="localGeminiModelPresetJson"
            rows="5"
            class="w-full px-3 py-2 rounded-lg bg-gray-100 dark:bg-dark-tertiary dark:border-dark-border dark:text-dark-text-primary border border-transparent focus:bg-white dark:focus:bg-dark-tertiary focus:border-apple-blue dark:focus:border-accent-blue focus:ring-2 focus:ring-apple-blue focus:ring-opacity-20 transition-all duration-200 outline-none font-mono text-xs"
          />
          <div class="text-xs text-apple-text-secondary dark:text-dark-text-secondary">{{ t('advancedGeminiModelPresetTip') }}</div>
          <div v-if="advancedGeminiPresetError" class="text-xs text-red-500">{{ advancedGeminiPresetError }}</div>
        </div>

        <div class="text-xs text-amber-600 dark:text-amber-400 bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 rounded-lg px-3 py-2">
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
import { applyProxyConfig, loadConfig, saveConfig, startProxy, stopProxy, saveLang } from './bridge/configBridge'
import type { AnthropicModelMapping, CodexEffortCapabilityMap, ConverterType, EndpointOption, GeminiModelPreset, ProxyConfigV2 } from './types/configTypes'
import { DEFAULT_PROXY_CONFIG_V2 } from './types/configTypes'

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

// 暗黑模式状态管理
const isDarkMode = ref(false)

// 初始化暗黑模式
onMounted(() => {
  // 从 localStorage 读取用户偏好
  const savedTheme = localStorage.getItem('theme')
  if (savedTheme === 'dark') {
    isDarkMode.value = true
  } else if (savedTheme === 'light') {
    isDarkMode.value = false
  } else {
    // 如果没有保存的偏好，使用系统偏好
    isDarkMode.value = window.matchMedia('(prefers-color-scheme: dark)').matches
  }
})

// 监听暗黑模式变化，保存到 localStorage
watch(isDarkMode, (newValue) => {
  localStorage.setItem('theme', newValue ? 'dark' : 'light')
})

// 切换暗黑模式的方法
const toggleDarkMode = () => {
  isDarkMode.value = !isDarkMode.value
}

const isRunning = ref(false)
const showLogs = ref(false)
const showAbout = ref(false)
const showSettings = ref(false)
const showAdvancedSettings = ref(false)
const showImportExport = ref(false)
const showEndpointDialog = ref(false)

const localMaxConcurrency = ref<number | null>(null)
const localLbModelCooldownSeconds = ref<number | null>(3600)
const localLbTransientBackoffSeconds = ref<number | null>(6)
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
const lbAvailabilityMap = ref<Record<string, boolean>>({})
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
let restartApplyTimer: ReturnType<typeof setTimeout> | null = null
let lastAppliedConfigHash = ''

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
  converter: 'codex' as ConverterType,
  codexModel: 'gpt-5.3-codex',
  codexModelMapping: {
    opus: 'gpt-5.3-codex',
    sonnet: 'gpt-5.2-codex',
    haiku: 'gpt-5.1-codex-mini',
  },
  anthropicModelMapping: {
    opus: '',
    sonnet: '',
    haiku: '',
  } as AnthropicModelMapping,
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
  lbModelCooldownSeconds: 3600,
  lbTransientBackoffSeconds: 6,
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
  ...DEFAULT_PROXY_CONFIG_V2,
  endpointOptions: [...DEFAULT_CONFIG.endpointOptions],
  selectedEndpointId: DEFAULT_CONFIG.selectedEndpointId,
  maxConcurrency: DEFAULT_CONFIG.maxConcurrency,
  lbModelCooldownSeconds: DEFAULT_CONFIG.lbModelCooldownSeconds,
  lbTransientBackoffSeconds: DEFAULT_CONFIG.lbTransientBackoffSeconds,
  ignoreProbeRequests: DEFAULT_CONFIG.ignoreProbeRequests,
  allowCountTokensFallbackEstimate: DEFAULT_CONFIG.allowCountTokensFallbackEstimate,
  reasoningEffort: { ...DEFAULT_CONFIG.reasoningEffort },
  converter: DEFAULT_CONFIG.converter,
  codexModel: DEFAULT_CONFIG.codexModel,
  codexModelMapping: { ...DEFAULT_CONFIG.codexModelMapping },
  anthropicModelMapping: { ...DEFAULT_CONFIG.anthropicModelMapping },
  codexEffortCapabilityMap: JSON.parse(JSON.stringify(DEFAULT_CONFIG.codexEffortCapabilityMap)),
  geminiModelPreset: [...DEFAULT_CONFIG.geminiModelPreset],
  geminiReasoningEffort: { ...DEFAULT_CONFIG.geminiReasoningEffort },
  loadBalancer: {
    ...DEFAULT_PROXY_CONFIG_V2.loadBalancer,
    lbProfiles: [...DEFAULT_PROXY_CONFIG_V2.loadBalancer.lbProfiles],
    lbEndpointConfigs: { ...DEFAULT_PROXY_CONFIG_V2.loadBalancer.lbEndpointConfigs },
  },
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

const normalizeAnthropicModelMapping = (input: unknown): AnthropicModelMapping => {
  const fallback = { ...DEFAULT_CONFIG.anthropicModelMapping }
  if (!input || typeof input !== 'object' || Array.isArray(input)) {
    return fallback
  }

  const source = input as Record<string, unknown>
  const normalizeItem = (value: unknown) => (typeof value === 'string' ? value.trim() : '')

  return {
    opus: normalizeItem(source.opus),
    sonnet: normalizeItem(source.sonnet),
    haiku: normalizeItem(source.haiku),
  }
}

const normalizeConverter = (value: unknown, fallback: ConverterType): ConverterType => {
  if (value === 'codex' || value === 'gemini' || value === 'anthropic') {
    return value
  }
  return fallback
}

const resolveSavedDefaultConverter = (savedConfig: ProxyConfigV2): ConverterType => {
  const configConverter = normalizeConverter(savedConfig.converter, DEFAULT_CONFIG.converter)
  const endpointConverter = savedConfig.endpointOptions
    ?.find((item) => item.id === savedConfig.selectedEndpointId)
    ?.converter
  return normalizeConverter(endpointConverter, configConverter)
}

const normalizeLoadBalancerConfig = (
  input: ProxyConfigV2['loadBalancer'],
  fallbackConverter: ConverterType = DEFAULT_CONFIG.converter,
) => {
  const fallback = DEFAULT_PROXY_CONFIG_V2.loadBalancer
  const normalizedConverter = normalizeConverter(fallbackConverter, DEFAULT_CONFIG.converter)
  const rawProfiles = Array.isArray(input?.lbProfiles) ? input.lbProfiles : [...fallback.lbProfiles]
  const lbProfiles = rawProfiles.map((profile) => {
    const normalizeSlot = (items: any[] | undefined) => (
      Array.isArray(items)
        ? items.map((candidate) => ({
          ...candidate,
          converterOverride: normalizeConverter(candidate?.converterOverride, normalizedConverter),
        }))
        : []
    )

    return {
      ...profile,
      modelMapping: {
        opus: normalizeSlot((profile as any)?.modelMapping?.opus),
        sonnet: normalizeSlot((profile as any)?.modelMapping?.sonnet),
        haiku: normalizeSlot((profile as any)?.modelMapping?.haiku),
      },
    }
  })
  const lbEndpointConfigs = input?.lbEndpointConfigs && typeof input.lbEndpointConfigs === 'object'
    ? input.lbEndpointConfigs
    : { ...fallback.lbEndpointConfigs }
  const selectedLbProfileId = input?.selectedLbProfileId
    && lbProfiles.some((item) => item.id === input.selectedLbProfileId)
    ? input.selectedLbProfileId
    : lbProfiles[0]?.id

  return {
    ...fallback,
    ...input,
    lbProfiles,
    lbEndpointConfigs,
    selectedLbProfileId,
  }
}

const updateSkillInjectionPrompt = (prompt: string) => {
  form.skillInjectionPrompt = prompt
  persistConfig(buildProxyConfig())
}

const openAdvancedSettings = () => {
  localMaxConcurrency.value = form.maxConcurrency
  localLbModelCooldownSeconds.value = form.lbModelCooldownSeconds
  localLbTransientBackoffSeconds.value = form.lbTransientBackoffSeconds
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
  form.lbModelCooldownSeconds = Math.max(1, Math.floor(localLbModelCooldownSeconds.value ?? DEFAULT_CONFIG.lbModelCooldownSeconds))
  form.lbTransientBackoffSeconds = Math.max(1, Math.floor(localLbTransientBackoffSeconds.value ?? DEFAULT_CONFIG.lbTransientBackoffSeconds))
  form.ignoreProbeRequests = localIgnoreProbeRequests.value
  form.allowCountTokensFallbackEstimate = localAllowCountTokensFallbackEstimate.value
  showAdvancedSettings.value = false
  persistConfig(buildProxyConfig())
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
    const savedDefaultConverter = resolveSavedDefaultConverter(savedConfig as ProxyConfigV2)
    // V2 defaults (non-breaking for old configs)
    form.proxyMode = (savedConfig as ProxyConfigV2).proxyMode || DEFAULT_PROXY_CONFIG_V2.proxyMode
    form.loadBalancer = normalizeLoadBalancerConfig(
      (savedConfig as ProxyConfigV2).loadBalancer,
      savedDefaultConverter,
    )
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
        anthropicModelMapping: item.anthropicModelMapping
          ? normalizeAnthropicModelMapping(item.anthropicModelMapping)
          : item.anthropicModelMapping,
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
    if (typeof savedConfig.lbModelCooldownSeconds === 'number') {
      form.lbModelCooldownSeconds = Math.max(1, Math.floor(savedConfig.lbModelCooldownSeconds))
    }
    if (typeof savedConfig.lbTransientBackoffSeconds === 'number') {
      form.lbTransientBackoffSeconds = Math.max(1, Math.floor(savedConfig.lbTransientBackoffSeconds))
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
    form.anthropicModelMapping = savedConfig.anthropicModelMapping
      ? normalizeAnthropicModelMapping(savedConfig.anthropicModelMapping)
      : { ...DEFAULT_CONFIG.anthropicModelMapping }
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
  form.anthropicModelMapping = endpoint.anthropicModelMapping
    ? normalizeAnthropicModelMapping(endpoint.anthropicModelMapping)
    : { ...DEFAULT_CONFIG.anthropicModelMapping }
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
  persistConfig(buildProxyConfig())
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
      anthropicModelMapping: { ...form.anthropicModelMapping },
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
    () => form.anthropicModelMapping,
    () => form.codexEffortCapabilityMap,
    () => form.geminiModelPreset,
    () => form.reasoningEffort,
    () => form.geminiReasoningEffort,
  ],
  () => {
    if (isSyncing.value) return
    updateSelectedEndpointConfig()
    persistConfig(buildProxyConfig())
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
      anthropicModelMapping: endpointData.anthropicModelMapping || { ...form.anthropicModelMapping },
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
    persistConfig(buildProxyConfig())
  }
  closeEndpointDialog()
}

const shouldShowLog = (message: string) => {
  if (message.startsWith('[Stat]')) return false
  if (message.includes('[Error]')) return true
  if (message.includes('[Warn]') || message.includes('[Warning]')) return true
  if (message.startsWith('[Req]')) return true
  if (message.startsWith('[Route]')) return true
  if (message.startsWith('[LBStatus]')) return true
  if (message.startsWith('[LB]')) return true
  if (message.startsWith('[ReqPayload]')) return true
  if (message.startsWith('[RateLimit]')) return true
  if (message.startsWith('[Tokens]')) return true
  if (message.includes('[System] Init success')) return true
  if (message.includes('Runtime config hot-updated')) return true
  if (message.includes('[Request] Sending request')) return true
  return false
}

const pushLog = (message: string) => {
  if (!shouldShowLog(message)) return
  const nextLog: LogItem = {
    time: new Date().toLocaleTimeString(),
    content: message,
  }
  logs.value = [...logs.value, nextLog].slice(-50)
}

const tryCountModelStat = (message: string) => {
  if (!message.startsWith('[Stat] model_request:')) return
  const family = message.replace('[Stat] model_request:', '').trim()
  if (family === 'opus') modelRequestStats.opus += 1
  if (family === 'sonnet') modelRequestStats.sonnet += 1
  if (family === 'haiku') modelRequestStats.haiku += 1
}

const tryUpdateLbAvailability = (message: string) => {
  if (!message.startsWith('[LBStatus]')) return

  const payload = message.replace('[LBStatus]', '').trim()
  if (!payload) return

  const kv: Record<string, string> = {}
  payload.split(/\s+/).forEach((token) => {
    const index = token.indexOf('=')
    if (index <= 0) return
    const key = token.slice(0, index)
    const value = token.slice(index + 1)
    if (key) kv[key] = value
  })

  const routeKey = kv.key
  const state = (kv.state || '').toLowerCase()
  if (!routeKey || !state) return

  const nextAvailability = state !== 'unavailable'
  lbAvailabilityMap.value = {
    ...lbAvailabilityMap.value,
    [routeKey]: nextAvailability,
  }
}

const resetDefaults = () => {
  form.port = DEFAULT_CONFIG.port
  form.endpointOptions = [...DEFAULT_CONFIG.endpointOptions]
  form.selectedEndpointId = DEFAULT_CONFIG.selectedEndpointId
  syncEndpointFromSelection()
  form.converter = DEFAULT_CONFIG.converter
  form.codexModel = DEFAULT_CONFIG.codexModel
  form.codexModelMapping = { ...DEFAULT_CONFIG.codexModelMapping }
  form.anthropicModelMapping = { ...DEFAULT_CONFIG.anthropicModelMapping }
  form.codexEffortCapabilityMap = JSON.parse(JSON.stringify(DEFAULT_CONFIG.codexEffortCapabilityMap))
  form.geminiModelPreset = [...DEFAULT_CONFIG.geminiModelPreset]
  form.reasoningEffort = { ...DEFAULT_CONFIG.reasoningEffort }
  form.geminiReasoningEffort = { ...DEFAULT_CONFIG.geminiReasoningEffort }
  form.maxConcurrency = DEFAULT_CONFIG.maxConcurrency
  form.lbModelCooldownSeconds = DEFAULT_CONFIG.lbModelCooldownSeconds
  form.lbTransientBackoffSeconds = DEFAULT_CONFIG.lbTransientBackoffSeconds
  form.ignoreProbeRequests = DEFAULT_CONFIG.ignoreProbeRequests
  form.allowCountTokensFallbackEstimate = DEFAULT_CONFIG.allowCountTokensFallbackEstimate
  form.proxyMode = DEFAULT_PROXY_CONFIG_V2.proxyMode
  form.loadBalancer = {
    ...DEFAULT_PROXY_CONFIG_V2.loadBalancer,
    lbProfiles: [...DEFAULT_PROXY_CONFIG_V2.loadBalancer.lbProfiles],
    lbEndpointConfigs: { ...DEFAULT_PROXY_CONFIG_V2.loadBalancer.lbEndpointConfigs },
  }
  useDefaultPrompt()
}

const buildProxyConfig = (force = false): ProxyConfigV2 => ({
  port: form.port,
  targetUrl: form.targetUrl,
  apiKey: form.apiKey,
  endpointOptions: form.endpointOptions,
  selectedEndpointId: form.selectedEndpointId,
  converter: form.converter,
  codexModel: form.codexModel,
  codexModelMapping: form.codexModelMapping,
  anthropicModelMapping: form.anthropicModelMapping,
  codexEffortCapabilityMap: form.codexEffortCapabilityMap,
  geminiModelPreset: form.geminiModelPreset,
  maxConcurrency: form.maxConcurrency,
  lbModelCooldownSeconds: form.lbModelCooldownSeconds,
  lbTransientBackoffSeconds: form.lbTransientBackoffSeconds,
  ignoreProbeRequests: form.ignoreProbeRequests,
  allowCountTokensFallbackEstimate: form.allowCountTokensFallbackEstimate,
  reasoningEffort: form.reasoningEffort,
  geminiReasoningEffort: form.geminiReasoningEffort,
  skillInjectionPrompt: form.skillInjectionPrompt,
  lang: locale.value,
  force,
  proxyMode: form.proxyMode,
  loadBalancer: form.loadBalancer,
})

const configRuntimeHash = (config: ProxyConfigV2) => JSON.stringify(config)

const scheduleApplyRunningConfig = (config: ProxyConfigV2) => {
  if (!isRunning.value) return

  const nextConfig = { ...config, force: false }
  const nextHash = configRuntimeHash(nextConfig)
  if (nextHash === lastAppliedConfigHash) return

  if (restartApplyTimer) {
    clearTimeout(restartApplyTimer)
  }

  restartApplyTimer = setTimeout(() => {
    if (!isRunning.value) return
    applyProxyConfig(nextConfig)
      .then(() => {
        lastAppliedConfigHash = nextHash
      })
      .catch(console.error)
  }, 350)
}

const persistConfig = (config: ProxyConfigV2 = buildProxyConfig()) => {
  saveConfig(config).catch(console.error)
  scheduleApplyRunningConfig(config)
}

const toggleProxy = () => {
  if (isRunning.value) {
    if (restartApplyTimer) {
      clearTimeout(restartApplyTimer)
      restartApplyTimer = null
    }
    lastAppliedConfigHash = ''
    stopProxy().catch(console.error)
  } else {
    const config = buildProxyConfig(false)
    startProxy(config)
      .then(() => {
        lastAppliedConfigHash = configRuntimeHash(config)
      })
      .catch(console.error)
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
        const savedDefaultConverter = resolveSavedDefaultConverter(savedConfig as ProxyConfigV2)
        if (savedConfig.port) form.port = savedConfig.port
        if (savedConfig.codexModel) {
          form.codexModel = migrateCodexModel(savedConfig.codexModel)
        }
        if (savedConfig.proxyMode) {
          form.proxyMode = savedConfig.proxyMode
        }
        form.loadBalancer = normalizeLoadBalancerConfig(
          savedConfig.loadBalancer,
          savedDefaultConverter,
        )
        if (savedConfig.endpointOptions && savedConfig.endpointOptions.length > 0) {
          form.endpointOptions = savedConfig.endpointOptions.map((item) => ({
            ...item,
            codexModel: item.codexModel ? migrateCodexModel(item.codexModel) : item.codexModel,
            codexModelMapping: item.codexModelMapping
              ? migrateCodexModelMapping(item.codexModelMapping)
              : item.codexModelMapping,
            anthropicModelMapping: item.anthropicModelMapping
              ? normalizeAnthropicModelMapping(item.anthropicModelMapping)
              : item.anthropicModelMapping,
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
        if (typeof savedConfig.lbModelCooldownSeconds === 'number') {
          form.lbModelCooldownSeconds = Math.max(1, Math.floor(savedConfig.lbModelCooldownSeconds))
        }
        if (typeof savedConfig.lbTransientBackoffSeconds === 'number') {
          form.lbTransientBackoffSeconds = Math.max(1, Math.floor(savedConfig.lbTransientBackoffSeconds))
        }
        if (savedConfig.codexModelMapping) {
          form.codexModelMapping = migrateCodexModelMapping({
            opus: savedConfig.codexModelMapping.opus || DEFAULT_CONFIG.codexModelMapping.opus,
            sonnet: savedConfig.codexModelMapping.sonnet || DEFAULT_CONFIG.codexModelMapping.sonnet,
            haiku: savedConfig.codexModelMapping.haiku || DEFAULT_CONFIG.codexModelMapping.haiku,
          })
        }
        form.anthropicModelMapping = savedConfig.anthropicModelMapping
          ? normalizeAnthropicModelMapping(savedConfig.anthropicModelMapping)
          : { ...DEFAULT_CONFIG.anthropicModelMapping }
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
        localLbModelCooldownSeconds.value = form.lbModelCooldownSeconds
        localLbTransientBackoffSeconds.value = form.lbTransientBackoffSeconds
        localIgnoreProbeRequests.value = form.ignoreProbeRequests
        localAllowCountTokensFallbackEstimate.value = form.allowCountTokensFallbackEstimate
      } else {
        syncEndpointFromSelection()
        useDefaultPrompt()
        persistConfig(buildProxyConfig())
      }
    })
    .catch(console.error)

  // Listen for proxy status
  listen<string>('proxy-status', (event) => {
    isRunning.value = event.payload === 'running'
    if (!isRunning.value) {
      lastAppliedConfigHash = ''
      if (restartApplyTimer) {
        clearTimeout(restartApplyTimer)
        restartApplyTimer = null
      }
    } else if (!lastAppliedConfigHash) {
      lastAppliedConfigHash = configRuntimeHash(buildProxyConfig(false))
    }
  }).then(unlisten => unlisteners.push(unlisten))

  // Listen for proxy logs
  listen<string>('proxy-log', (event) => {
    const message = event.payload
    tryCountModelStat(message)
    tryUpdateLbAvailability(message)
    pushLog(message)
  }).then(unlisten => unlisteners.push(unlisten))

  // Listen for port-in-use
  listen<number>('port-in-use', (event) => {
    const port = event.payload
    const confirmed = confirm(
      t('portInUse', { port })
    )
    if (confirmed) {
      const config = buildProxyConfig(true)
      startProxy(config)
        .then(() => {
          lastAppliedConfigHash = configRuntimeHash({ ...config, force: false })
        })
        .catch(console.error)
    } else {
      isRunning.value = false
    }
  }).then(unlisten => unlisteners.push(unlisten))
})

onUnmounted(() => {
  if (restartApplyTimer) {
    clearTimeout(restartApplyTimer)
    restartApplyTimer = null
  }
  unlisteners.forEach(unlisten => unlisten())
})

watch(
  [
    () => form.maxConcurrency,
    () => form.lbModelCooldownSeconds,
    () => form.lbTransientBackoffSeconds,
    () => form.ignoreProbeRequests,
    () => form.allowCountTokensFallbackEstimate,
    () => form.proxyMode,
    () => form.loadBalancer,
  ],
  () => {
    if (!isSyncing.value) {
      persistConfig(buildProxyConfig())
    }
  },
  { deep: true }
)
</script>

<style scoped>
.app-container {
  min-height: 100vh;
  background-color: #f5f5f7;
  font-family: -apple-system, BlinkMacSystemFont, "SF Pro Text", "Helvetica Neue", Arial, sans-serif;
}

.app-container.dark {
  background-color: #0a0a0f;
}

.main-content {
  max-width: 600px;
  margin: 0 auto;
  padding: 40px 20px;
}
</style>
