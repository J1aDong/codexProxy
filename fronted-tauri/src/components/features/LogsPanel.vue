<template>
  <div>
    <!-- Backdrop -->
    <Transition name="fade">
      <div
        v-if="visible"
        class="fixed inset-0 bg-black/20 z-40"
        @click="handleClose"
      ></div>
    </Transition>

    <div class="fixed inset-y-0 right-0 w-96 bg-white dark:bg-dark-secondary shadow-2xl z-50 flex flex-col transform transition-transform duration-300 ease-in-out"
         :class="{ 'translate-x-full': !visible, 'translate-x-0': visible }">
      <div class="flex items-center justify-between p-4 border-b border-gray-200 dark:border-dark-border">
        <h2 class="text-lg font-semibold text-apple-text-primary dark:text-dark-text-primary">{{ t('logsTitle') }}</h2>
        <Button
          type="text"
          size="small"
          circle
          @click="handleClose"
        >
          <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </Button>
      </div>

      <div class="p-4 border-b border-gray-200 dark:border-dark-border">
        <div class="flex items-center gap-3 text-sm">
          <span class="flex items-center gap-1.5">
            <span class="bg-red-500 text-white text-xs font-medium px-2 py-0.5 rounded-md">Opus</span>
            <span class="text-apple-text-primary dark:text-dark-text-primary font-medium">{{ modelRequestStats.opus }}</span>
          </span>
          <span class="flex items-center gap-1.5">
            <span class="bg-green-500 text-white text-xs font-medium px-2 py-0.5 rounded-md">Sonnet</span>
            <span class="text-apple-text-primary dark:text-dark-text-primary font-medium">{{ modelRequestStats.sonnet }}</span>
          </span>
          <span class="flex items-center gap-1.5">
            <span class="bg-gray-800 text-white text-xs font-medium px-2 py-0.5 rounded-md">Haiku</span>
            <span class="text-apple-text-primary dark:text-dark-text-primary font-medium">{{ modelRequestStats.haiku }}</span>
          </span>
        </div>
      </div>

      <div class="flex-1 overflow-y-auto p-4 overscroll-contain flex flex-col" ref="logsContainer">
        <div v-if="logs.length === 0" class="flex-1 flex items-center justify-center text-apple-text-secondary dark:text-dark-text-secondary min-h-[50px]">
          {{ t('noLogs') }}
        </div>
        <div v-else>
          <div v-for="(log, index) in logs" :key="index" class="mb-2">
            <div class="flex gap-2 items-center">
              <span class="w-[64px] shrink-0 text-xs text-apple-text-secondary dark:text-dark-text-secondary tabular-nums">{{ log.time }}</span>
              <span
                v-if="extractLogTag(log.content)"
                class="text-[10px] px-1.5 py-0.5 rounded border shrink-0"
                :class="getLogTagClass(extractLogTag(log.content))"
              >
                [{{ extractLogTag(log.content) }}]
              </span>
            </div>
            <div class="mt-1 ml-[72px] text-xs text-apple-text-primary dark:text-dark-text-primary break-all leading-5">
              {{ extractLogBody(log.content) }}
            </div>
          </div>
        </div>
      </div>

      <div class="p-4 border-t border-gray-200 dark:border-dark-border flex justify-end">
        <Button @click="handleClear">{{ t('clearLogs') }}</Button>
      </div>
    </div>
  </div>
</template>

<script lang="ts" setup>
import { ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Button from '../base/Button.vue'

const { t } = useI18n()

interface LogItem {
  time: string
  content: string
}

interface ModelStats {
  opus: number
  sonnet: number
  haiku: number
}

const props = defineProps({
  visible: {
    type: Boolean,
    required: true,
  },
  logs: {
    type: Array as () => LogItem[],
    required: true,
  },
  modelRequestStats: {
    type: Object as () => ModelStats,
    required: true,
  },
})

const emit = defineEmits(['close', 'clear'])

const logsContainer = ref<HTMLElement | null>(null)

watch(() => props.logs, () => {
  if (props.visible && logsContainer.value) {
    setTimeout(() => {
      if (logsContainer.value) {
        logsContainer.value.scrollTop = logsContainer.value.scrollHeight
      }
    }, 0)
  }
})

const handleClose = () => {
  emit('close')
}

const handleClear = () => {
  emit('clear')
}

const parseLog = (content: string) => {
  const match = content.match(/^\[([^\]]+)\]\s*(.*)$/)
  if (!match) {
    return { tag: '', body: content }
  }
  return {
    tag: match[1],
    body: match[2] || '',
  }
}

const extractLogTag = (content: string) => parseLog(content).tag

const extractLogBody = (content: string) => parseLog(content).body

const getLogTagClass = (tag: string) => {
  const normalized = tag.toLowerCase()
  if (normalized === 'req') return 'bg-green-50 dark:bg-green-900/30 text-green-700 dark:text-green-400 border-green-200 dark:border-green-700'
  if (normalized === 'warn' || normalized === 'warning' || normalized === 'error') {
    return 'bg-red-50 dark:bg-red-900/30 text-red-700 dark:text-red-400 border-red-200 dark:border-red-700'
  }
  if (normalized === 'route') return 'bg-blue-50 dark:bg-blue-900/30 text-blue-700 dark:text-blue-400 border-blue-200 dark:border-blue-700'
  if (normalized === 'tokens' || normalized === 'ratelimit') return 'bg-amber-50 dark:bg-amber-900/30 text-amber-700 dark:text-amber-400 border-amber-200 dark:border-amber-700'
  return 'bg-gray-100 dark:bg-gray-800 text-gray-700 dark:text-gray-300 border-gray-200 dark:border-gray-600'
}
</script>

<style scoped>
.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.3s ease;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}
</style>
