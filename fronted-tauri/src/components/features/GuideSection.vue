<template>
  <div
    class="rounded-xl shadow-sm p-6 transition-colors duration-300"
    :class="isDarkMode ? 'bg-dark-secondary border border-dark-border' : 'bg-white'"
  >
    <h3 class="text-xs font-semibold uppercase tracking-wider text-apple-text-secondary dark:text-dark-text-secondary mb-4">
      {{ t('guideTitle') }}
    </h3>
    <p v-if="clientMode === 'claude'" class="text-sm text-apple-text-primary dark:text-dark-text-primary mb-4 leading-relaxed">
      {{ t('guideDesc') }}<br />
      <code class="bg-gray-200 dark:bg-gray-700 px-1.5 py-0.5 rounded text-xs font-mono">~/.claude/settings.json</code>
    </p>
    <p v-else class="text-sm text-apple-text-primary dark:text-dark-text-primary mb-4 leading-relaxed">
      {{ t('guideCodexDesc') }}<br />
      <code class="bg-gray-200 dark:bg-gray-700 px-1.5 py-0.5 rounded text-xs font-mono">~/.codex/config.toml</code>
    </p>
    <div class="relative bg-gray-900 dark:bg-gray-800 rounded-xl p-4 mb-4">
      <pre class="font-mono text-xs text-gray-300 dark:text-gray-200 whitespace-pre-wrap leading-relaxed">
{{ configExample }}
      </pre>
      <div class="absolute top-3 right-3 flex gap-2">
        <Button
          type="link"
          size="small"
          :label="copied ? t('copied') : t('copy')"
          @click="handleCopy"
        />
      </div>
    </div>
    <div class="flex items-center gap-3 flex-wrap">
      <Button
        type="primary"
        size="small"
        :label="applyStatus === 'loading' ? t('applyingConfig') : t('applyConfig')"
        :disabled="applyStatus === 'loading'"
        @click="handleApply"
      />
      <span v-if="applyStatus === 'success'" class="text-xs text-green-600 dark:text-green-400">
        {{ t('applyConfigSuccess', { path: appliedPath }) }}
      </span>
      <span v-else-if="applyStatus === 'error'" class="text-xs text-red-600 dark:text-red-400">
        {{ t('applyConfigFailed', { error: applyError }) }}
      </span>
    </div>
  </div>
</template>

<script lang="ts" setup>
import { ref, computed } from 'vue'
import { useI18n } from 'vue-i18n'
import Button from '../base/Button.vue'
import { applyClaudeConfig, applyCodexConfig } from '../../bridge/configBridge'

const { t } = useI18n()

const props = defineProps({
  port: {
    type: Number,
    required: true,
  },
  isDarkMode: {
    type: Boolean,
    required: true,
  },
  clientMode: {
    type: String as () => 'claude' | 'codex',
    default: 'claude',
  },
})

const copied = ref(false)
const applyStatus = ref<'idle' | 'loading' | 'success' | 'error'>('idle')
const appliedPath = ref('')
const applyError = ref('')

const configExample = computed(() => {
  if (props.clientMode === 'codex') {
    return `model_provider = "codex-proxy"

[model_providers.codex-proxy]
name = "Codex Proxy"
base_url = "http://localhost:${props.port}/codex/v1"
env_key = "OPENAI_API_KEY"
wire_api = "responses"`
  }

  const tokenPlaceholder = t('guideTokenHint')

  return `{
  "env": {
    "ANTHROPIC_BASE_URL": "http://localhost:${props.port}",
    "ANTHROPIC_AUTH_TOKEN": "${tokenPlaceholder}"
  },
  "forceLoginMethod": "claudeai",
  "permissions": {
    "allow": [],
    "deny": []
  }
}`
})

const handleCopy = async () => {
  try {
    await navigator.clipboard.writeText(configExample.value)
    copied.value = true
    setTimeout(() => {
      copied.value = false
    }, 2000)
  } catch (error) {
    console.error('Copy failed:', error)
  }
}

const handleApply = async () => {
  applyStatus.value = 'loading'
  applyError.value = ''
  appliedPath.value = ''
  try {
    const path =
      props.clientMode === 'codex'
        ? await applyCodexConfig(props.port)
        : await applyClaudeConfig(props.port, 'proxy-configured')
    appliedPath.value = path
    applyStatus.value = 'success'
    setTimeout(() => {
      if (applyStatus.value === 'success') applyStatus.value = 'idle'
    }, 4000)
  } catch (error) {
    applyError.value = error instanceof Error ? error.message : String(error)
    applyStatus.value = 'error'
  }
}
</script>
