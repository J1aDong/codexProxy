<template>
  <div class="bg-white rounded-xl shadow-sm p-6 mb-8">
    <div class="grid grid-cols-1 md:grid-cols-2 gap-5">
      <div>
        <div class="flex items-center h-8 mb-1">
          <label class="block text-sm font-medium text-apple-text-primary">端口</label>
        </div>
        <Input
          v-model="localPort"
          :label="''"
          placeholder="8889"
          @blur="handlePortChange"
        />
      </div>
      <div>
        <div class="flex items-center justify-between h-8 mb-1">
          <label class="block text-sm font-medium text-apple-text-primary">{{ t.targetUrl }}</label>
          <Button
            type="primary"
            size="small"
            circle
            @click="handleAddEndpoint"
          >
            +
          </Button>
        </div>
        <Select
          v-model="form.selectedEndpointId"
          :options="endpointSelectOptions"
          placeholder="选择目标地址"
          @change="handleEndpointChange"
        />
      </div>
    </div>

    <div class="mt-5">
      <Input
        v-model="localApiKey"
        label="Codex API 密钥"
        type="password"
        placeholder="选填 - 将覆盖客户端提供的密钥"
        tip="如果在此处配置，您可以在 Claude Code 中使用任意随机字符串作为 API 密钥。"
        @blur="handleApiKeyChange"
      />
    </div>

    <div class="mt-5">
      <Select
        v-model="form.codexModel"
        :options="modelOptions"
        label="Codex 模型"
      />
    </div>

    <div class="mt-5 pt-4 border-t border-gray-200">
      <h3 class="text-sm font-semibold text-apple-text-primary mb-3">{{ t.reasoningEffort }}</h3>
      <div class="grid grid-cols-1 md:grid-cols-3 gap-5">
        <div>
          <Select
            v-model="form.reasoningEffort.opus"
            :options="effortOptions"
            label="Opus"
          />
        </div>
        <div>
          <Select
            v-model="form.reasoningEffort.sonnet"
            :options="effortOptions"
            label="Sonnet"
          />
        </div>
        <div>
          <Select
            v-model="form.reasoningEffort.haiku"
            :options="effortOptions"
            label="Haiku"
          />
        </div>
      </div>
      <div class="text-apple-text-secondary text-xs mt-2">
        {{ t.reasoningEffortTip }}
      </div>
    </div>

    <div class="mt-6 pt-4 border-t border-gray-200 flex justify-between items-center">
      <Button @click="handleReset">{{ t.restoreDefaults }}</Button>
      <Button
        :type="isRunning ? 'danger' : 'primary'"
        :label="isRunning ? t.stopProxy : t.startProxy"
        class="min-w-[120px]"
        @click="handleToggle"
      />
    </div>
  </div>
</template>

<script lang="ts" setup>
import { ref, computed } from 'vue'
import Button from '../base/Button.vue'
import Input from '../base/Input.vue'
import Select from '../base/Select.vue'

interface EndpointOption {
  id: string
  alias: string
  url: string
  apiKey: string
}

interface FormData {
  port: number
  targetUrl: string
  apiKey: string
  endpointOptions: EndpointOption[]
  selectedEndpointId: string
  codexModel: string
  reasoningEffort: {
    opus: string
    sonnet: string
    haiku: string
  }
  skillInjectionPrompt: string
}

const props = defineProps({
  form: {
    type: Object as () => FormData,
    required: true,
  },
  isRunning: {
    type: Boolean,
    required: true,
  },
  t: {
    type: Object,
    required: true,
  },
})

const emit = defineEmits(['update:form', 'reset', 'toggle', 'addEndpoint'])

const localPort = ref(props.form.port)
const localApiKey = ref(props.form.apiKey)

const modelOptions = [
  { value: 'gpt-5.3-codex', label: 'GPT-5.3-Codex (推荐)' },
  { value: 'gpt-5.2-codex', label: 'GPT-5.2-Codex' },
]

const effortOptions = [
  { value: 'low', label: 'Low' },
  { value: 'medium', label: 'Medium' },
  { value: 'high', label: 'High' },
  { value: 'xhigh', label: 'Extra High' },
]

const endpointSelectOptions = computed(() => {
  return props.form.endpointOptions.map(option => ({
    value: option.id,
    label: option.alias,
  }))
})

const handlePortChange = () => {
  const port = Number(localPort.value)
  if (!isNaN(port) && port > 0 && port <= 65535) {
    emit('update:form', {
      ...props.form,
      port,
    })
  }
}

const handleApiKeyChange = () => {
  emit('update:form', {
    ...props.form,
    apiKey: localApiKey.value,
  })
}

const handleEndpointChange = (id: string) => {
  const endpoint = props.form.endpointOptions.find(opt => opt.id === id)
  if (endpoint) {
    const newForm = {
      ...props.form,
      selectedEndpointId: id,
      targetUrl: endpoint.url,
      apiKey: endpoint.apiKey,
    }
    emit('update:form', newForm)
    localApiKey.value = endpoint.apiKey
  }
}

const handleReset = () => {
  emit('reset')
}

const handleToggle = () => {
  emit('toggle')
}

const handleAddEndpoint = () => {
  emit('addEndpoint')
}
</script>
