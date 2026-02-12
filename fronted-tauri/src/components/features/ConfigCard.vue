<template>
  <div class="bg-white rounded-xl shadow-sm p-6 mb-8">
    <div class="grid grid-cols-1 md:grid-cols-2 gap-5">
      <div>
        <div class="flex items-center h-8 mb-1">
          <label class="block text-sm font-medium text-apple-text-primary">{{ t('port') }}</label>
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
          <label class="block text-sm font-medium text-apple-text-primary">{{ t('targetUrl') }}</label>
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
          :placeholder="t('selectTargetUrl')"
          @change="handleEndpointChange"
        >
          <template #option="{ option }">
            <span>{{ option.label }}</span>
            <button 
              class="text-gray-400 hover:text-apple-blue opacity-0 group-hover:opacity-100 transition-all duration-200 p-1 rounded-full hover:bg-blue-50 focus:outline-none"
              @click.stop="handleEditEndpoint(option.value)"
              :title="t('edit')"
            >
              <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15.232 5.232l3.536 3.536m-2.036-5.036a2.5 2.5 0 113.536 3.536L6.5 21.036H3v-3.572L16.732 3.732z" />
              </svg>
            </button>
          </template>
        </Select>
      </div>
    </div>

    <div class="mt-5">
      <Input
        v-model="localApiKey"
        :label="t('apiKey')"
        type="password"
        :placeholder="t('apiKeyPlaceholder')"
        :tip="t('apiKeyTip')"
        @blur="handleApiKeyChange"
      />
    </div>

    <div class="mt-5">
      <Select
        v-model="form.converter"
        :options="converterOptions"
        :label="t('converter')"
      />
    </div>

    <div class="mt-5 pt-4 border-t border-gray-200">
      <h3 class="text-sm font-semibold text-apple-text-primary mb-3">{{ t('reasoningEffort') }}</h3>
      <div class="grid grid-cols-1 md:grid-cols-3 gap-5">
        <div>
          <Select
            v-if="form.converter === 'gemini'"
            v-model="form.geminiReasoningEffort.opus"
            :options="geminiModelOptions"
            label="Opus"
          />
          <Select
            v-else
            v-model="form.codexModelMapping.opus"
            :options="codexModelOptions"
            label="Opus"
          />
          <div v-if="form.converter === 'codex'" class="mt-2">
            <Select
              v-model="form.reasoningEffort.opus"
              :options="codexEffortOptionsBySlot.opus"
              :label="t('effortLevel')"
            />
          </div>
        </div>
        <div>
          <Select
            v-if="form.converter === 'gemini'"
            v-model="form.geminiReasoningEffort.sonnet"
            :options="geminiModelOptions"
            label="Sonnet"
          />
          <Select
            v-else
            v-model="form.codexModelMapping.sonnet"
            :options="codexModelOptions"
            label="Sonnet"
          />
          <div v-if="form.converter === 'codex'" class="mt-2">
            <Select
              v-model="form.reasoningEffort.sonnet"
              :options="codexEffortOptionsBySlot.sonnet"
              :label="t('effortLevel')"
            />
          </div>
        </div>
        <div>
          <Select
            v-if="form.converter === 'gemini'"
            v-model="form.geminiReasoningEffort.haiku"
            :options="geminiModelOptions"
            label="Haiku"
          />
          <Select
            v-else
            v-model="form.codexModelMapping.haiku"
            :options="codexModelOptions"
            label="Haiku"
          />
          <div v-if="form.converter === 'codex'" class="mt-2">
            <Select
              v-model="form.reasoningEffort.haiku"
              :options="codexEffortOptionsBySlot.haiku"
              :label="t('effortLevel')"
            />
          </div>
        </div>
      </div>
      <div class="text-apple-text-secondary text-xs mt-2">
        {{ form.converter === 'gemini' ? t('geminiReasoningEffortTip') : t('reasoningEffortTip') }}
      </div>
    </div>

    <div class="mt-6 pt-4 border-t border-gray-200 flex justify-between items-center">
      <Button @click="handleReset">{{ t('restoreDefaults') }}</Button>
      <Button
        :type="isRunning ? 'danger' : 'primary'"
        :label="isRunning ? t('stopProxy') : t('startProxy')"
        class="min-w-[120px]"
        @click="handleToggle"
      />
    </div>
  </div>
</template>

<script lang="ts" setup>
import { ref, computed, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Button from '../base/Button.vue'
import Input from '../base/Input.vue'
import Select from '../base/Select.vue'

const { t } = useI18n()

interface EndpointOption {
  id: string
  alias: string
  url: string
  apiKey: string
}

type ConverterType = 'codex' | 'gemini'

interface FormData {
  port: number
  targetUrl: string
  apiKey: string
  endpointOptions: EndpointOption[]
  selectedEndpointId: string
  converter: ConverterType
  codexModelMapping: {
    opus: string
    sonnet: string
    haiku: string
  }
  codexEffortCapabilityMap: Record<string, string[]>
  geminiModelPreset: string[]
  reasoningEffort: {
    opus: string
    sonnet: string
    haiku: string
  }
  geminiReasoningEffort: {
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
})

const emit = defineEmits(['update:form', 'reset', 'toggle', 'addEndpoint', 'editEndpoint'])

const localPort = ref(props.form.port)
const localApiKey = ref(props.form.apiKey)

watch(() => props.form.apiKey, (newVal) => {
  localApiKey.value = newVal
})

const codexModelOptions = computed(() => [
  { value: 'gpt-5.3-codex', label: t('modelRecommended') },
  { value: 'gpt-5.2-codex', label: 'GPT-5.2-Codex' },
  { value: 'gpt-5-codex', label: 'GPT-5-Codex' },
  { value: 'gpt-5.1-codex-max', label: 'GPT-5.1-Codex-Max' },
  { value: 'gpt-5.1-codex', label: 'GPT-5.1-Codex' },
  { value: 'gpt-5.1-codex-mini', label: 'GPT-5.1-Codex-Mini' },
])

const geminiModelOptions = computed(() => [
  ...props.form.geminiModelPreset.map((model) => ({
    value: model,
    label: model,
  })),
])

const converterOptions = computed(() => [
  { value: 'codex', label: t('converterCodex') },
  { value: 'gemini', label: t('converterGemini') },
])

const effortLabelMap: Record<string, string> = {
  low: 'Low',
  medium: 'Medium',
  high: 'High',
  xhigh: 'Extra High',
}

const toEffortOptions = (efforts: string[]) =>
  efforts.map((effort) => ({ value: effort, label: effortLabelMap[effort] || effort }))

const codexEffortOptionsBySlot = computed(() => {
  const getSlotOptions = (model: string) => {
    const capabilities = props.form.codexEffortCapabilityMap[model] || ['medium', 'high']
    return toEffortOptions(capabilities)
  }

  return {
    opus: getSlotOptions(props.form.codexModelMapping.opus),
    sonnet: getSlotOptions(props.form.codexModelMapping.sonnet),
    haiku: getSlotOptions(props.form.codexModelMapping.haiku),
  }
})

const ensureEffortCompatible = (slot: 'opus' | 'sonnet' | 'haiku') => {
  const model = props.form.codexModelMapping[slot]
  const currentEffort = props.form.reasoningEffort[slot]
  const allowed = props.form.codexEffortCapabilityMap[model] || ['medium', 'high']
  if (!allowed.includes(currentEffort)) {
    emit('update:form', {
      ...props.form,
      reasoningEffort: {
        ...props.form.reasoningEffort,
        [slot]: allowed[0],
      },
    })
  }
}

const ensureGeminiModelCompatible = (slot: 'opus' | 'sonnet' | 'haiku') => {
  const allowed = props.form.geminiModelPreset
  if (allowed.length === 0) return

  const currentModel = props.form.geminiReasoningEffort[slot]
  if (!allowed.includes(currentModel)) {
    emit('update:form', {
      ...props.form,
      geminiReasoningEffort: {
        ...props.form.geminiReasoningEffort,
        [slot]: allowed[0],
      },
    })
  }
}

watch(() => props.form.codexModelMapping.opus, () => ensureEffortCompatible('opus'))
watch(() => props.form.codexModelMapping.sonnet, () => ensureEffortCompatible('sonnet'))
watch(() => props.form.codexModelMapping.haiku, () => ensureEffortCompatible('haiku'))
watch(
  () => props.form.geminiModelPreset,
  () => {
    ensureGeminiModelCompatible('opus')
    ensureGeminiModelCompatible('sonnet')
    ensureGeminiModelCompatible('haiku')
  },
  { deep: true }
)

watch(
  () => [
    props.form.codexModelMapping.opus,
    props.form.codexModelMapping.sonnet,
    props.form.codexModelMapping.haiku,
  ],
  () => {
    emit('update:form', {
      ...props.form,
      codexModel: props.form.codexModelMapping.sonnet,
    })
  }
)

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

const handleEditEndpoint = (id: string | number) => {
  emit('editEndpoint', String(id))
}
</script>
