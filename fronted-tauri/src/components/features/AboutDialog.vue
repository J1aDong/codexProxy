<template>
  <Dialog
    :visible="visible"
    :title="t('aboutTitle')"
    @close="handleClose"
  >
    <div class="text-center">
      <div class="text-lg font-semibold text-apple-text-primary dark:text-dark-text-primary mb-2">{{ t('appName') }}</div>
      <div class="text-sm text-apple-text-secondary dark:text-dark-text-secondary mb-4">
        {{ t('versionLabel') }} v{{ appVersion }}
      </div>

      <div class="mb-4">
        <div class="text-xs text-apple-text-secondary dark:text-dark-text-secondary mb-2">{{ updateStatusText }}</div>
        <Button
          type="primary"
          size="small"
          plain
          @click="handleOpenReleases"
        >
          {{ t('goToReleases') }}
        </Button>
      </div>
    </div>

    <template #footer>
      <div class="p-4 flex justify-end">
        <Button type="primary" @click="handleClose">{{ t('ok') }}</Button>
      </div>
    </template>
  </Dialog>
</template>

<script lang="ts" setup>
import { computed, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Button from '../base/Button.vue'
import Dialog from '../base/Dialog.vue'

const { t } = useI18n()

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
})

const emit = defineEmits(['close', 'checkUpdate', 'openReleases'])

const updateStatusText = computed(() => {
  if (props.updateStatus === 'checking') return t('updateChecking')
  if (props.updateStatus === 'failed') {
    if (props.updateError === 'rate_limited') return t('updateRateLimited')
    return props.updateError
      ? `${t('updateFailed')} (${props.updateError})`
      : t('updateFailed')
  }
  if (props.updateStatus === 'available') {
    return `${t('updateAvailable')} v${props.latestVersion}`
  }
  if (props.updateStatus === 'latest') return t('updateLatest')
  return t('updateIdle')
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
