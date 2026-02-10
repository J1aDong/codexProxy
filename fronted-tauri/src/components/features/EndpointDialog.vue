<template>
  <div v-if="visible" class="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
    <div class="bg-white rounded-xl w-full max-w-md mx-4">
      <div class="flex items-center justify-between p-4 border-b border-gray-200">
        <h2 class="text-lg font-semibold text-apple-text-primary">{{ t.addEndpoint }}</h2>
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
          label="别名"
          placeholder="例如：自建节点"
        />
        <Input
          v-model="endpointDraft.url"
          label="地址"
          placeholder="https://..."
        />
        <Input
          v-model="endpointDraft.apiKey"
          label="密钥"
          type="password"
          placeholder="选填 - 将覆盖客户端提供的密钥"
        />
      </div>

      <div class="p-4 border-t border-gray-200 flex justify-end gap-2">
        <Button @click="handleClose">{{ t.cancel }}</Button>
        <Button type="primary" @click="handleAdd">{{ t.add }}</Button>
      </div>
    </div>
  </div>
</template>

<script lang="ts" setup>
import { reactive } from 'vue'
import Button from '../base/Button.vue'
import Input from '../base/Input.vue'

defineProps({
  visible: {
    type: Boolean,
    required: true,
  },
  t: {
    type: Object,
    required: true,
  },
})

const emit = defineEmits(['close', 'add'])

const endpointDraft = reactive({
  alias: '',
  url: '',
  apiKey: '',
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
