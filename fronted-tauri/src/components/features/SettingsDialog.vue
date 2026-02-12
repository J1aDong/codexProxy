<template>
  <Dialog
    :visible="visible"
    :title="t('settingsTitle')"
    @close="handleClose"
  >
    <div class="space-y-4">
      <div>
        <label class="block text-sm font-medium text-apple-text-primary mb-1">
          {{ t('skillInjection') }}
        </label>
        <textarea
          v-model="localPrompt"
          rows="4"
          class="w-full px-3 py-2.5 rounded-lg bg-gray-100 border border-transparent focus:bg-white focus:border-apple-blue focus:ring-2 focus:ring-apple-blue focus:ring-opacity-20 transition-all duration-200 outline-none resize-none"
          :placeholder="t('skillInjectionPlaceholder')"
          maxlength="500"
        />
        <div class="text-apple-text-secondary text-xs mt-1">
          {{ t('skillInjectionTip') }}
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

const { t, locale } = useI18n()

const props = defineProps({
  visible: {
    type: Boolean,
    required: true,
  },
  skillInjectionPrompt: {
    type: String,
    required: true,
  },
})

const emit = defineEmits(['close', 'update'])

const localPrompt = ref(props.skillInjectionPrompt)

watch(() => props.visible, (val) => {
  if (val) {
    localPrompt.value = props.skillInjectionPrompt
  }
})

const DEFAULT_PROMPT_ZH = "skills里的技能如果需要依赖，先安装，不要先用其他方案，如果还有问题告知用户解决方案让用户选择"
const DEFAULT_PROMPT_EN = "If skills require dependencies, install them first. Do not use workarounds. If issues persist, provide solutions for the user to choose."

const handleClose = () => {
  emit('close')
}

const handleUseDefault = () => {
  localPrompt.value = locale.value === 'zh' ? DEFAULT_PROMPT_ZH : DEFAULT_PROMPT_EN
}

const handleSave = () => {
  emit('update', localPrompt.value)
  handleClose()
}
</script>
