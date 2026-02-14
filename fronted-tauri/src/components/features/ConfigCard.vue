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
        <div class="flex items-center h-8 mb-1">
          <label class="block text-sm font-medium text-apple-text-primary">{{ t('proxyModeLabel') }}</label>
        </div>
        <Select
          :model-value="form.proxyMode"
          :options="proxyModeOptions"
          @change="handleProxyModeChange"
        />
      </div>
    </div>

    <div class="mt-5">
      <transition name="proxy-mode-fade" mode="out-in">
        <div :key="form.proxyMode">
          <div v-if="form.proxyMode === 'single'">
            <div class="grid grid-cols-1 md:grid-cols-2 gap-5">
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
              <div>
                <div class="flex items-center h-8 mb-1">
                  <label class="block text-sm font-medium text-apple-text-primary">{{ t('apiKey') }}</label>
                </div>
                <Input
                  v-model="localApiKey"
                  :label="''"
                  type="password"
                  :placeholder="t('apiKeyPlaceholder')"
                  :tip="t('apiKeyTip')"
                  @blur="handleApiKeyChange"
                />
              </div>
            </div>

            <div class="mt-5">
              <Select
                v-model="form.converter"
                :options="converterOptions"
                :label="t('converter')"
              />
            </div>

            <div v-if="form.converter !== 'anthropic'" class="mt-5 pt-4 border-t border-gray-200">
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

            <div v-else class="mt-5 pt-4 border-t border-gray-200">
              <h3 class="text-sm font-semibold text-apple-text-primary mb-3">{{ t('anthropicModelMappingTitle') }}</h3>
              <div class="grid grid-cols-1 md:grid-cols-3 gap-5">
                <Input
                  v-model="form.anthropicModelMapping.opus"
                  label="Opus"
                  :placeholder="t('anthropicModelPlaceholder')"
                />
                <Input
                  v-model="form.anthropicModelMapping.sonnet"
                  label="Sonnet"
                  :placeholder="t('anthropicModelPlaceholder')"
                />
                <Input
                  v-model="form.anthropicModelMapping.haiku"
                  label="Haiku"
                  :placeholder="t('anthropicModelPlaceholder')"
                />
              </div>
              <div class="text-apple-text-secondary text-xs mt-2">
                {{ t('anthropicModelMappingTip') }}
              </div>
              <div class="text-apple-text-secondary text-xs mt-1">
                {{ t('anthropicPassthroughTip') }}
              </div>
            </div>
          </div>

          <div v-else class="space-y-4">
            <div class="flex items-center justify-between gap-2 mb-3">
              <div class="text-sm font-semibold text-apple-text-primary">{{ t('lbProfile') }}</div>
              <div class="flex items-center gap-2">
                <Button type="primary" size="small" @click="handleAddLbProfile">+</Button>
                <Button size="small" @click="handleRenameLbProfile">{{ t('rename') }}</Button>
                <Button size="small" type="danger" @click="handleDeleteLbProfile">{{ t('delete') }}</Button>
              </div>
            </div>

            <Select
              v-model="selectedLbProfileId"
              :options="lbProfileOptions"
              :placeholder="t('lbProfileSelect')"
            />

            <div
              v-if="currentLbProfile"
              class="space-y-4"
            >
              <div class="text-sm font-semibold text-apple-text-primary">{{ t('lbModelMapping') }}</div>

              <div
                v-for="slot in lbModelSlots"
                :key="slot"
                class="space-y-2"
              >
                <div class="text-xs text-apple-text-secondary mb-2 uppercase">{{ slot }}</div>

                <div
                  v-for="(candidate, idx) in getSlotCandidates(slot)"
                  :key="`${slot}-${idx}-${candidate.endpointId}`"
                  class="p-2 rounded-lg border border-gray-200 bg-gray-50"
                >
                  <div class="flex items-center gap-2">
                    <span
                      class="inline-block w-2.5 h-2.5 rounded-full shrink-0"
                      :class="isSlotCandidateUnavailable(slot, candidate) ? 'bg-red-500' : 'bg-green-500'"
                      :title="isSlotCandidateUnavailable(slot, candidate) ? t('lbStatusUnavailable') : t('lbStatusAvailable')"
                    ></span>
                    <Select
                      class="flex-1"
                      :model-value="candidate.endpointId"
                      :options="endpointOptionsForSelect"
                      :placeholder="t('lbEndpoint')"
                      @change="(value) => handleUpdateSlotCandidateEndpoint(slot, idx, String(value))"
                    />
                    <Button
                      size="small"
                      @click="toggleSlotCandidateExpanded(slot, idx, candidate)"
                    >
                      {{ isSlotCandidateExpanded(slot, idx, candidate) ? t('collapse') : t('edit') }}
                    </Button>
                    <Button size="small" @click="handleMoveSlotCandidate(slot, idx, -1)">↑</Button>
                    <Button size="small" @click="handleMoveSlotCandidate(slot, idx, 1)">↓</Button>
                    <Button size="small" type="danger" @click="handleDeleteSlotCandidate(slot, idx)">-</Button>
                  </div>

                  <div class="mt-2 text-xs text-apple-text-secondary">
                    {{ getSlotCandidateSummary(slot, candidate) }}
                  </div>

                  <div
                    v-if="isSlotCandidateExpanded(slot, idx, candidate)"
                    class="mt-2 pt-2 border-t border-gray-200"
                  >
                    <div class="grid grid-cols-1 md:grid-cols-3 gap-2">
                      <Select
                        :model-value="getCandidateConverterValue(candidate)"
                        :options="lbConverterOptions"
                        :label="t('lbConverter')"
                        @change="(value) => handleUpdateSlotCandidateConverter(slot, idx, String(value))"
                      />

                      <template v-if="getEffectiveSlotCandidateConverter(candidate) === 'codex'">
                        <Select
                          :model-value="getCandidateModelValue(slot, candidate)"
                          :options="codexModelOptions"
                          :label="t('lbModel')"
                          @change="(value) => handleUpdateSlotCandidateModel(slot, idx, String(value))"
                        />
                        <Select
                          :model-value="getCandidateReasoningEffortValue(slot, candidate)"
                          :options="getCodexEffortOptionsByCandidate(slot, candidate)"
                          :label="t('lbCodexEffort')"
                          @change="(value) => handleUpdateSlotCandidateReasoningEffort(slot, idx, String(value))"
                        />
                      </template>

                      <template v-else-if="getEffectiveSlotCandidateConverter(candidate) === 'gemini'">
                        <Select
                          class="md:col-span-2"
                          :model-value="getCandidateModelValue(slot, candidate)"
                          :options="geminiModelOptions"
                          :label="t('lbModel')"
                          @change="(value) => handleUpdateSlotCandidateModel(slot, idx, String(value))"
                        />
                      </template>

                      <template v-else>
                        <Input
                          class="md:col-span-2"
                          :model-value="candidate.customModelName || ''"
                          :label="t('lbModel')"
                          :placeholder="t('lbCustomModelPlaceholder')"
                          @update:modelValue="(value) => handleUpdateSlotCandidateModel(slot, idx, String(value))"
                        />
                      </template>
                    </div>

                    <div
                      v-if="getEffectiveSlotCandidateConverter(candidate) === 'anthropic'"
                      class="text-xs text-apple-text-secondary mt-2"
                    >
                      {{ t('lbAnthropicModelTip') }}
                    </div>
                  </div>
                </div>

                <Button
                  size="small"
                  type="primary"
                  :disabled="endpointOptionsForSelect.length === 0"
                  @click="handleAddSlotCandidate(slot)"
                >
                  + {{ t('add') }}
                </Button>
              </div>
            </div>
          </div>
        </div>
      </transition>
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

    <Dialog
      :visible="isRenamingLbProfile"
      :title="t('lbRenameDialogTitle')"
      @close="handleCancelRenameLbProfile"
    >
      <Input
        v-model="lbProfileRenameDraft"
        :label="t('lbProfile')"
      />

      <template #footer>
        <div class="p-4 flex justify-end gap-2">
          <Button @click="handleCancelRenameLbProfile">{{ t('cancel') }}</Button>
          <Button type="primary" @click="handleConfirmRenameLbProfile">{{ t('save') }}</Button>
        </div>
      </template>
    </Dialog>
  </div>
</template>

<script lang="ts" setup>
import { ref, computed, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Button from '../base/Button.vue'
import Dialog from '../base/Dialog.vue'
import Input from '../base/Input.vue'
import Select from '../base/Select.vue'
import type { ConverterType } from '../../types/configTypes'
import type {
  LbConverterType,
  LbSlotEndpointRef,
  LoadBalancerProfile,
  ProxyMode,
} from '../../types/loadBalancerTypes'

const { t } = useI18n()

interface EndpointOption {
  id: string
  alias: string
  url: string
  apiKey: string
  converter?: ConverterType
}

interface FormData {
  port: number
  targetUrl: string
  apiKey: string
  endpointOptions: EndpointOption[]
  selectedEndpointId: string
  converter: ConverterType
  proxyMode: ProxyMode
  loadBalancer: {
    lbProfiles: LoadBalancerProfile[]
    selectedLbProfileId?: string
    lbEndpointConfigs: Record<string, {
      endpointId: string
      enabled: boolean
      maxConcurrency: number
      priority: number
      weight: number
    }>
  }
  codexModelMapping: {
    opus: string
    sonnet: string
    haiku: string
  }
  anthropicModelMapping: {
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
  lbAvailabilityMap: {
    type: Object as () => Record<string, boolean>,
    default: () => ({}),
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
  { value: 'anthropic', label: t('converterAnthropic') },
])

const proxyModeOptions = computed(() => [
  { value: 'single', label: t('proxyModeSingle') },
  { value: 'load_balancer', label: t('proxyModeLoadBalancer') },
])

const lbConverterOptions = computed(() => [
  { value: 'codex', label: t('converterCodex') },
  { value: 'gemini', label: t('converterGemini') },
  { value: 'anthropic', label: t('converterAnthropic') },
])

const formatLbProfileName = (name: string) => {
  if (name === 'Default LB Profile') return t('defaultLbProfileName')
  const profileMatch = name.match(/^Profile (\d+)$/)
  if (profileMatch) {
    return t('lbProfileNameWithIndex', { index: profileMatch[1] })
  }
  return name
}

const selectedLbProfileId = computed({
  get: () => props.form.loadBalancer.selectedLbProfileId || '',
  set: (value: string) => {
    emit('update:form', {
      ...props.form,
      loadBalancer: {
        ...props.form.loadBalancer,
        selectedLbProfileId: value || undefined,
      },
    })
  },
})

const lbProfileOptions = computed(() => {
  return props.form.loadBalancer.lbProfiles.map((profile) => ({
    value: profile.id,
    label: formatLbProfileName(profile.name),
  }))
})

const isRenamingLbProfile = ref(false)
const lbProfileRenameDraft = ref('')

type ModelSlot = 'opus' | 'sonnet' | 'haiku'

const lbModelSlots: ModelSlot[] = ['opus', 'sonnet', 'haiku']
const expandedSlotCandidates = ref<Record<string, boolean>>({})

const toLbConverter = (value: string): LbConverterType => {
  if (value === 'gemini' || value === 'anthropic') return value
  return 'codex'
}

const getSlotCandidateKey = (slot: ModelSlot, idx: number, candidate: LbSlotEndpointRef) => {
  const profileId = props.form.loadBalancer.selectedLbProfileId || 'no-profile'
  return `${profileId}:${slot}:${idx}:${candidate.endpointId}`
}

const isSlotCandidateExpanded = (slot: ModelSlot, idx: number, candidate: LbSlotEndpointRef) => {
  return expandedSlotCandidates.value[getSlotCandidateKey(slot, idx, candidate)] === true
}

const toggleSlotCandidateExpanded = (slot: ModelSlot, idx: number, candidate: LbSlotEndpointRef) => {
  const key = getSlotCandidateKey(slot, idx, candidate)
  const current = expandedSlotCandidates.value[key] === true
  expandedSlotCandidates.value = {
    ...expandedSlotCandidates.value,
    [key]: !current,
  }
}

const createDefaultProfile = (name: string): LoadBalancerProfile => {
  const firstEndpointId = props.form.endpointOptions[0]?.id || ''
  const defaultConverter = toLbConverter(props.form.converter)
  const buildDefaultCandidate = (slot: ModelSlot): LbSlotEndpointRef => {
    const candidate: LbSlotEndpointRef = {
      endpointId: firstEndpointId,
      converterOverride: defaultConverter,
    }
    return normalizeCandidateByConverter(slot, candidate, defaultConverter)
  }

  return {
    id: `lb-profile-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
    name,
    description: '',
    modelMapping: {
      opus: firstEndpointId ? [buildDefaultCandidate('opus')] : [],
      sonnet: firstEndpointId ? [buildDefaultCandidate('sonnet')] : [],
      haiku: firstEndpointId ? [buildDefaultCandidate('haiku')] : [],
    },
    strategy: {
      errorThreshold: 5,
      errorWindowSeconds: 60,
      cooldownSeconds: 3600,
      degradedConcurrency: 4,
    },
  }
}

const handleProxyModeChange = (mode: string) => {
  const nextMode = mode === 'load_balancer' ? 'load_balancer' : 'single'
  const hasProfiles = props.form.loadBalancer.lbProfiles.length > 0
  const nextProfiles = hasProfiles
    ? props.form.loadBalancer.lbProfiles
    : [createDefaultProfile(t('defaultLbProfileName'))]
  const nextSelected = props.form.loadBalancer.selectedLbProfileId
    || nextProfiles[0]?.id
    || undefined

  emit('update:form', {
    ...props.form,
    proxyMode: nextMode,
    loadBalancer: {
      ...props.form.loadBalancer,
      lbProfiles: nextProfiles,
      selectedLbProfileId: nextSelected,
    },
  })
}

const handleAddLbProfile = () => {
  const nextProfile = createDefaultProfile(
    t('lbProfileNameWithIndex', { index: props.form.loadBalancer.lbProfiles.length + 1 }),
  )
  emit('update:form', {
    ...props.form,
    loadBalancer: {
      ...props.form.loadBalancer,
      lbProfiles: [...props.form.loadBalancer.lbProfiles, nextProfile],
      selectedLbProfileId: nextProfile.id,
    },
  })
}

const handleRenameLbProfile = () => {
  const selectedId = props.form.loadBalancer.selectedLbProfileId
  if (!selectedId) return
  const current = props.form.loadBalancer.lbProfiles.find((item) => item.id === selectedId)
  if (!current) return

  isRenamingLbProfile.value = true
  lbProfileRenameDraft.value = current.name === 'Default LB Profile'
    ? t('defaultLbProfileName')
    : current.name
}

const handleConfirmRenameLbProfile = () => {
  const selectedId = props.form.loadBalancer.selectedLbProfileId
  const draftName = lbProfileRenameDraft.value.trim()
  const nextName = draftName === t('defaultLbProfileName')
    ? 'Default LB Profile'
    : draftName
  if (!selectedId || !nextName) {
    handleCancelRenameLbProfile()
    return
  }

  emit('update:form', {
    ...props.form,
    loadBalancer: {
      ...props.form.loadBalancer,
      lbProfiles: props.form.loadBalancer.lbProfiles.map((item) => {
        if (item.id !== selectedId) return item
        return { ...item, name: nextName }
      }),
    },
  })

  isRenamingLbProfile.value = false
}

const handleCancelRenameLbProfile = () => {
  isRenamingLbProfile.value = false
  lbProfileRenameDraft.value = ''
}

const handleDeleteLbProfile = () => {
  const selectedId = props.form.loadBalancer.selectedLbProfileId
  if (!selectedId) return
  if (props.form.loadBalancer.lbProfiles.length <= 1) return

  const nextProfiles = props.form.loadBalancer.lbProfiles.filter((item) => item.id !== selectedId)
  emit('update:form', {
    ...props.form,
    loadBalancer: {
      ...props.form.loadBalancer,
      lbProfiles: nextProfiles,
      selectedLbProfileId: nextProfiles[0]?.id,
    },
  })
}

const endpointOptionsForSelect = computed(() => (
  props.form.endpointOptions.map((item) => ({
    value: item.id,
    label: item.alias,
  }))
))

const currentLbProfile = computed(() => {
  const selectedId = props.form.loadBalancer.selectedLbProfileId
  if (!selectedId) return null
  return props.form.loadBalancer.lbProfiles.find((item) => item.id === selectedId) || null
})

const updateCurrentLbProfile = (updater: (profile: LoadBalancerProfile) => LoadBalancerProfile) => {
  const selectedId = props.form.loadBalancer.selectedLbProfileId
  if (!selectedId) return

  const nextProfiles = props.form.loadBalancer.lbProfiles.map((item) => {
    if (item.id !== selectedId) return item
    return updater(item)
  })

  emit('update:form', {
    ...props.form,
    loadBalancer: {
      ...props.form.loadBalancer,
      lbProfiles: nextProfiles,
    },
  })
}

const updateSlotCandidate = (
  slot: ModelSlot,
  idx: number,
  updater: (candidate: LbSlotEndpointRef) => LbSlotEndpointRef,
) => {
  updateCurrentLbProfile((profile) => ({
    ...profile,
    modelMapping: {
      ...profile.modelMapping,
      [slot]: profile.modelMapping[slot].map((candidate, index) => {
        if (index !== idx) return candidate
        return updater(candidate)
      }),
    },
  }))
}

const getSlotCandidates = (slot: ModelSlot) => {
  const profile = currentLbProfile.value
  if (!profile) return []
  return profile.modelMapping[slot]
}

const getEndpointConverter = (endpointId: string): LbConverterType => {
  const endpoint = props.form.endpointOptions.find((item) => item.id === endpointId)
  return toLbConverter(endpoint?.converter || props.form.converter)
}

const getEffectiveSlotCandidateConverter = (candidate: LbSlotEndpointRef): LbConverterType => {
  return toLbConverter(candidate.converterOverride || getEndpointConverter(candidate.endpointId))
}

const sanitizeRouteKeyToken = (value: string): string => {
  return value.trim().replace(/\s+/g, '_').replace(/\|/g, '_')
}

const getCandidateModelHint = (candidate: LbSlotEndpointRef): string => {
  const trimmed = (candidate.customModelName || '').trim()
  if (!trimmed) return '_default'
  return sanitizeRouteKeyToken(trimmed)
}

const getSlotCandidateRouteKey = (slot: ModelSlot, candidate: LbSlotEndpointRef): string => {
  const endpointId = sanitizeRouteKeyToken(candidate.endpointId)
  const converter = sanitizeRouteKeyToken(getEffectiveSlotCandidateConverter(candidate))
  const modelHint = getCandidateModelHint(candidate)
  return `${slot}|${endpointId}|${converter}|${modelHint}`
}

const isSlotCandidateUnavailable = (slot: ModelSlot, candidate: LbSlotEndpointRef): boolean => {
  const routeKey = getSlotCandidateRouteKey(slot, candidate)
  return props.lbAvailabilityMap[routeKey] === false
}

const getCandidateConverterValue = (candidate: LbSlotEndpointRef) => {
  return getEffectiveSlotCandidateConverter(candidate)
}

const getDefaultModelForSlot = (slot: ModelSlot, converter: LbConverterType): string => {
  if (converter === 'gemini') {
    return props.form.geminiReasoningEffort[slot]
      || props.form.geminiModelPreset[0]
      || ''
  }
  if (converter === 'codex') {
    return props.form.codexModelMapping[slot]
      || codexModelOptions.value[0]?.value
      || 'gpt-5.3-codex'
  }
  return ''
}

const getAllowedCodexEffortsByModel = (model: string): string[] => {
  return props.form.codexEffortCapabilityMap[model] || ['medium', 'high']
}

const normalizeCandidateByConverter = (
  slot: ModelSlot,
  candidate: LbSlotEndpointRef,
  converter: LbConverterType,
): LbSlotEndpointRef => {
  if (converter === 'codex') {
    const nextModel = candidate.customModelName || getDefaultModelForSlot(slot, 'codex')
    const allowed = getAllowedCodexEffortsByModel(nextModel)
    const slotFallback = props.form.reasoningEffort[slot]
    const nextEffort = candidate.customReasoningEffort && allowed.includes(candidate.customReasoningEffort)
      ? candidate.customReasoningEffort
      : allowed.includes(slotFallback)
        ? slotFallback
        : allowed[0]

    return {
      ...candidate,
      customModelName: nextModel,
      customReasoningEffort: nextEffort,
    }
  }

  if (converter === 'gemini') {
    const nextModel = candidate.customModelName || getDefaultModelForSlot(slot, 'gemini')
    return {
      ...candidate,
      customModelName: nextModel || undefined,
      customReasoningEffort: undefined,
    }
  }

  return {
    ...candidate,
    customReasoningEffort: undefined,
  }
}

const handleAddSlotCandidate = (slot: ModelSlot) => {
  const defaultEndpointId = props.form.endpointOptions[0]?.id
  if (!defaultEndpointId) return

  const defaultConverter = toLbConverter(props.form.converter)
  const rawCandidate: LbSlotEndpointRef = {
    endpointId: defaultEndpointId,
    converterOverride: defaultConverter,
  }

  const candidate = normalizeCandidateByConverter(
    slot,
    rawCandidate,
    defaultConverter,
  )

  updateCurrentLbProfile((profile) => ({
    ...profile,
    modelMapping: {
      ...profile.modelMapping,
      [slot]: [...profile.modelMapping[slot], candidate],
    },
  }))
}

const handleDeleteSlotCandidate = (slot: ModelSlot, idx: number) => {
  updateCurrentLbProfile((profile) => ({
    ...profile,
    modelMapping: {
      ...profile.modelMapping,
      [slot]: profile.modelMapping[slot].filter((_, index) => index !== idx),
    },
  }))
}

const handleMoveSlotCandidate = (slot: ModelSlot, idx: number, direction: -1 | 1) => {
  updateCurrentLbProfile((profile) => {
    const current = [...profile.modelMapping[slot]]
    const nextIndex = idx + direction
    if (nextIndex < 0 || nextIndex >= current.length) {
      return profile
    }

    const target = current[idx]
    current[idx] = current[nextIndex]
    current[nextIndex] = target

    return {
      ...profile,
      modelMapping: {
        ...profile.modelMapping,
        [slot]: current,
      },
    }
  })
}

const handleUpdateSlotCandidateEndpoint = (slot: ModelSlot, idx: number, endpointId: string) => {
  if (!endpointId) return

  updateSlotCandidate(slot, idx, (candidate) => ({
    ...candidate,
    endpointId,
  }))
}

const handleUpdateSlotCandidateConverter = (slot: ModelSlot, idx: number, converter: string) => {
  updateSlotCandidate(slot, idx, (candidate) => {
    const converterOverride = toLbConverter(converter)

    return normalizeCandidateByConverter(
      slot,
      {
        ...candidate,
        converterOverride,
      },
      converterOverride,
    )
  })
}

const getCandidateModelValue = (slot: ModelSlot, candidate: LbSlotEndpointRef): string => {
  const converter = getEffectiveSlotCandidateConverter(candidate)
  if (candidate.customModelName) return candidate.customModelName
  return getDefaultModelForSlot(slot, converter)
}

const handleUpdateSlotCandidateModel = (slot: ModelSlot, idx: number, model: string) => {
  updateSlotCandidate(slot, idx, (candidate) => {
    const effectiveConverter = getEffectiveSlotCandidateConverter(candidate)
    const nextCandidate: LbSlotEndpointRef = {
      ...candidate,
      customModelName: model.trim() ? model.trim() : undefined,
    }

    return normalizeCandidateByConverter(slot, nextCandidate, effectiveConverter)
  })
}

const getCodexEffortOptionsByCandidate = (slot: ModelSlot, candidate: LbSlotEndpointRef) => {
  const model = getCandidateModelValue(slot, candidate)
  const allowed = getAllowedCodexEffortsByModel(model)
  return allowed.map((effort) => ({
    value: effort,
    label: effortLabelMap[effort] || effort,
  }))
}

const getCandidateReasoningEffortValue = (slot: ModelSlot, candidate: LbSlotEndpointRef) => {
  const model = getCandidateModelValue(slot, candidate)
  const allowed = getAllowedCodexEffortsByModel(model)

  if (candidate.customReasoningEffort && allowed.includes(candidate.customReasoningEffort)) {
    return candidate.customReasoningEffort
  }

  if (allowed.includes(props.form.reasoningEffort[slot])) {
    return props.form.reasoningEffort[slot]
  }

  return allowed[0]
}

const getConverterLabel = (converter: LbConverterType) => {
  if (converter === 'gemini') return t('converterGemini')
  if (converter === 'anthropic') return t('converterAnthropic')
  return t('converterCodex')
}

const getSlotCandidateSummary = (slot: ModelSlot, candidate: LbSlotEndpointRef) => {
  const converter = getEffectiveSlotCandidateConverter(candidate)
  const converterLabel = getConverterLabel(converter)

  if (converter === 'codex') {
    const model = getCandidateModelValue(slot, candidate)
    const effort = getCandidateReasoningEffortValue(slot, candidate)
    const effortLabel = effortLabelMap[effort] || effort
    return `${converterLabel} · ${model} · ${effortLabel}`
  }

  if (converter === 'gemini') {
    const model = getCandidateModelValue(slot, candidate)
    return `${converterLabel} · ${model}`
  }

  const anthropicModel = (candidate.customModelName || '').trim() || t('anthropicModelPlaceholder')
  return `${converterLabel} · ${anthropicModel}`
}

const handleUpdateSlotCandidateReasoningEffort = (slot: ModelSlot, idx: number, effort: string) => {
  updateSlotCandidate(slot, idx, (candidate) => ({
    ...candidate,
    customReasoningEffort: effort,
  }))
}

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

const ensureEffortCompatible = (slot: ModelSlot) => {
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

const ensureGeminiModelCompatible = (slot: ModelSlot) => {
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
  { deep: true },
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
  },
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
  const endpoint = props.form.endpointOptions.find((opt) => opt.id === id)
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

<style scoped>
.proxy-mode-fade-enter-active,
.proxy-mode-fade-leave-active {
  transition: opacity 0.18s ease, transform 0.18s ease;
}

.proxy-mode-fade-enter-from,
.proxy-mode-fade-leave-to {
  opacity: 0;
  transform: translateX(6px);
}
</style>
