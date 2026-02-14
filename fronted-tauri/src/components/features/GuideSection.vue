<template>
  <div
    class="rounded-xl shadow-sm p-6 transition-colors duration-300"
    :class="isDarkMode ? 'bg-dark-secondary border border-dark-border' : 'bg-white'"
  >
    <h3 class="text-xs font-semibold uppercase tracking-wider text-apple-text-secondary dark:text-dark-text-secondary mb-4">
      {{ t('guideTitle') }}
    </h3>
    <p class="text-sm text-apple-text-primary dark:text-dark-text-primary mb-4 leading-relaxed">
      {{ t('guideDesc') }}<br />
      <code class="bg-gray-200 dark:bg-gray-700 px-1.5 py-0.5 rounded text-xs font-mono">~/.claude/settings.json</code>
    </p>
    <div class="relative bg-gray-900 dark:bg-gray-800 rounded-xl p-4 mb-4">
      <pre class="font-mono text-xs text-gray-300 dark:text-gray-200 whitespace-pre-wrap leading-relaxed">
{{ configExample }}
      </pre>
      <div class="absolute top-3 right-3">
        <Button
          type="link"
          size="small"
          :label="copied ? t('copied') : t('copy')"
          @click="handleCopy"
        />
      </div>
    </div>
  </div>
</template>

<script lang="ts" setup>
import { ref, computed } from 'vue'
import { useI18n } from 'vue-i18n'
import Button from '../base/Button.vue'

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
})

const copied = ref(false)

const configExample = computed(() => {
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
</script>
