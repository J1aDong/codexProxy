<template>
  <Dialog
    :visible="visible"
    :title="t('settingsTitle')"
    @close="handleClose"
  >
    <div class="space-y-4">
      <div>
        <label class="block text-sm font-medium text-apple-text-primary dark:text-dark-text-primary mb-1">
          {{ t('customInjection') }}
        </label>
        <textarea
          v-model="localPrompt"
          rows="4"
          class="w-full px-3 py-2.5 rounded-lg bg-gray-100 dark:bg-dark-secondary border border-transparent focus:bg-white dark:focus:bg-dark-primary focus:border-apple-blue focus:ring-2 focus:ring-apple-blue focus:ring-opacity-20 transition-all duration-200 outline-none resize-none text-apple-text-primary dark:text-dark-text-primary"
          :placeholder="t('customInjectionPlaceholder')"
          maxlength="500"
        />
        <div class="text-apple-text-secondary dark:text-dark-text-secondary text-xs mt-1">
          {{ t('customInjectionTip') }}
        </div>
        <Button
          type="link"
          size="small"
          @click="handleUseDefault"
          class="mt-2"
        >
          {{ t('useDefaultPrompt') }}
        </Button>
      </div>
    </div>

    <template #footer>
      <div class="p-4 flex justify-end">
        <Button type="primary" @click="handleSave">{{ t('ok') }}</Button>
      </div>
    </template>
  </Dialog>
</template>

<script lang="ts" setup>
import { ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Button from '../base/Button.vue'
import Dialog from '../base/Dialog.vue'

const { t } = useI18n()

const props = defineProps({
  visible: {
    type: Boolean,
    required: true,
  },
  customInjectionPrompt: {
    type: String,
    required: true,
  },
})

const emit = defineEmits(['close', 'update', 'useDefault'])

const localPrompt = ref(props.customInjectionPrompt)

watch(() => props.customInjectionPrompt, (val) => {
  localPrompt.value = val
})

const handleClose = () => {
  emit('close')
}

const handleUseDefault = () => {
  emit('useDefault')
}

const handleSave = () => {
  emit('update', localPrompt.value)
  handleClose()
}
</script>
