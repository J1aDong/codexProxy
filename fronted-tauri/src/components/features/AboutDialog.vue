<template>
  <div v-if="visible" class="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
    <div class="bg-white rounded-xl w-full max-w-md mx-4">
      <div class="flex items-center justify-between p-4 border-b border-gray-200">
        <h2 class="text-lg font-semibold text-apple-text-primary">{{ t.aboutTitle }}</h2>
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

      <div class="p-4 text-center">
        <div class="text-lg font-semibold text-apple-text-primary mb-2">{{ t.appName }}</div>
        <div class="text-sm text-apple-text-secondary mb-4">
          {{ t.versionLabel }} v{{ appVersion }}
        </div>

        <div class="mb-4">
          <div class="text-xs text-apple-text-secondary mb-2">{{ updateStatusText }}</div>
          <Button
            type="primary"
            size="small"
            plain
            @click="handleOpenReleases"
          >
            {{ t.goToReleases }}
          </Button>
        </div>
      </div>

      <div class="p-4 border-t border-gray-200 flex justify-end">
        <Button type="primary" @click="handleClose">OK</Button>
      </div>
    </div>
  </div>
</template>

<script lang="ts" setup>
import { computed, watch } from 'vue'
import Button from '../base/Button.vue'

const props = defineProps({
  visible: {
    type: Boolean,
    required: true,
  },
  appVersion: {
    type: String,
    required: true,
  },
  updateStatus: {
    type: String,
    required: true,
  },
  latestVersion: {
    type: String,
    required: true,
  },
  updateError: {
    type: String,
    required: true,
  },
  t: {
    type: Object,
    required: true,
  },
})

const emit = defineEmits(['close', 'checkUpdate', 'openReleases'])

const updateStatusText = computed(() => {
  const { updateStatus, updateError, latestVersion, t } = props
  if (updateStatus === 'checking') return t.updateChecking
  if (updateStatus === 'failed') {
    if (updateError === 'rate_limited') return t.updateRateLimited
    return updateError
      ? `${t.updateFailed} (${updateError})`
      : t.updateFailed
  }
  if (updateStatus === 'available') {
    return `${t.updateAvailable} v${latestVersion}`
  }
  if (updateStatus === 'latest') return t.updateLatest
  return t.updateIdle
})

watch(() => props.visible, (visible) => {
  if (visible && props.updateStatus !== 'checking') {
    emit('checkUpdate')
  }
})

const handleClose = () => {
  emit('close')
}

const handleOpenReleases = () => {
  emit('openReleases')
}
</script>
