<template>
  <div v-if="visible" class="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
    <div class="bg-white rounded-xl w-full max-w-md mx-4">
      <div class="flex items-center justify-between p-4 border-b border-gray-200">
        <h2 class="text-lg font-semibold text-apple-text-primary">{{ t.settingsTitle }}</h2>
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

      <div class="p-4 space-y-4">
        <div>
          <label class="block text-sm font-medium text-apple-text-primary mb-1">
            {{ t.skillInjection }}
          </label>
          <textarea
            v-model="localPrompt"
            rows="4"
            class="w-full px-3 py-2.5 rounded-lg bg-gray-100 border border-transparent focus:bg-white focus:border-apple-blue focus:ring-2 focus:ring-apple-blue focus:ring-opacity-20 transition-all duration-200 outline-none resize-none"
            :placeholder="t.skillInjectionPlaceholder"
            maxlength="500"
          />
          <div class="text-apple-text-secondary text-xs mt-1">
            {{ t.skillInjectionTip }}
          </div>
          <Button
            type="link"
            size="small"
            @click="handleUseDefault"
            class="mt-2"
          >
            {{ t.useDefaultPrompt }}
          </Button>
        </div>
      </div>

      <div class="p-4 border-t border-gray-200 flex justify-end">
        <Button type="primary" @click="handleSave">OK</Button>
      </div>
    </div>
  </div>
</template>

<script lang="ts" setup>
import { ref } from 'vue'
import Button from '../base/Button.vue'

const props = defineProps({
  visible: {
    type: Boolean,
    required: true,
  },
  skillInjectionPrompt: {
    type: String,
    required: true,
  },
  lang: {
    type: String,
    required: true,
  },
  t: {
    type: Object,
    required: true,
  },
})

const emit = defineEmits(['close', 'update'])

const localPrompt = ref(props.skillInjectionPrompt)

const DEFAULT_PROMPT_ZH = "skills里的技能如果需要依赖，先安装，不要先用其他方案，如果还有问题告知用户解决方案让用户选择"
const DEFAULT_PROMPT_EN = "If skills require dependencies, install them first. Do not use workarounds. If issues persist, provide solutions for the user to choose."

const handleClose = () => {
  emit('close')
}

const handleUseDefault = () => {
  localPrompt.value = props.lang === 'zh' ? DEFAULT_PROMPT_ZH : DEFAULT_PROMPT_EN
}

const handleSave = () => {
  emit('update', localPrompt.value)
  handleClose()
}
</script>
