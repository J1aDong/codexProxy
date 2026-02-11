<template>
  <div v-if="visible" class="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
    <div class="bg-white rounded-xl w-full max-w-md mx-4">
      <div class="flex items-center justify-between p-4 border-b border-gray-200">
        <h2 class="text-lg font-semibold text-apple-text-primary">{{ isEdit ? t('editEndpoint') : t('addEndpoint') }}</h2>
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
        <Input
          v-model="endpointDraft.alias"
          :label="t('endpointAlias')"
          :placeholder="t('endpointAliasPlaceholder')"
        />
        <Input
          v-model="endpointDraft.url"
          :label="t('endpointUrl')"
          placeholder="https://..."
        />
        <Input
          v-model="endpointDraft.apiKey"
          :label="t('endpointApiKey')"
          type="password"
          :placeholder="t('apiKeyPlaceholder')"
        />
      </div>

      <div class="p-4 border-t border-gray-200 flex justify-end gap-2">
        <Button @click="handleClose">{{ t('cancel') }}</Button>
        <Button type="primary" @click="handleAdd">{{ isEdit ? t('save') : t('add') }}</Button>
      </div>
    </div>
  </div>
</template>

<script lang="ts" setup>
import { reactive, watch, computed } from 'vue'
import { useI18n } from 'vue-i18n'
import Button from '../base/Button.vue'
import Input from '../base/Input.vue'

const { t } = useI18n()

const props = defineProps({
  visible: {
    type: Boolean,
    required: true,
  },
  initialData: {
    type: Object,
    default: null,
  },
})

const emit = defineEmits(['close', 'add'])

const endpointDraft = reactive({
  alias: '',
  url: '',
  apiKey: '',
})

const isEdit = computed(() => !!props.initialData)

watch(() => props.visible, (val) => {
  if (val) {
    if (props.initialData) {
      endpointDraft.alias = props.initialData.alias
      endpointDraft.url = props.initialData.url
      endpointDraft.apiKey = props.initialData.apiKey
    } else {
      endpointDraft.alias = ''
      endpointDraft.url = ''
      endpointDraft.apiKey = ''
    }
  }
})

const handleClose = () => {
  endpointDraft.alias = ''
  endpointDraft.url = ''
  endpointDraft.apiKey = ''
  emit('close')
}

const handleAdd = () => {
  const { alias, url, apiKey } = endpointDraft
  if (!alias.trim() || !url.trim()) {
    return
  }

  try {
    const parsed = new URL(url.trim())
    if (!['https:', 'http:'].includes(parsed.protocol)) {
      return
    }
  } catch {
    return
  }

  emit('add', {
    alias: alias.trim(),
    url: url.trim(),
    apiKey: apiKey.trim(),
  })
  handleClose()
}
</script>
