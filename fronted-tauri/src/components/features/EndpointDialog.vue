<template>
  <Dialog
    :visible="visible"
    :title="isEdit ? t('editEndpoint') : t('addEndpoint')"
    @close="handleClose"
  >
    <!-- Normal form content -->
    <div v-if="deleteStep === 0" class="space-y-4">
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

    <!-- Delete Confirmation UI -->
    <div v-else class="text-center space-y-4 py-2">
      <div class="w-12 h-12 bg-red-100 rounded-full flex items-center justify-center mx-auto text-red-500">
        <svg class="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" /></svg>
      </div>
      <div class="space-y-2">
        <h3 class="text-lg font-medium text-gray-900">
          {{ deleteStep === 1 ? t('confirmDeleteEndpoint', { name: endpointDraft.alias || endpointDraft.url }) : t('confirmDeleteEndpointFinal') }}
        </h3>
        <p class="text-sm text-gray-500" v-if="deleteStep === 2">
          This action cannot be undone.
        </p>
      </div>
    </div>

    <template #footer>
      <!-- Normal form footer -->
      <div v-if="deleteStep === 0" class="p-4 flex justify-between items-center gap-2">
        <div v-if="isEdit">
          <Button type="text" danger @click="handleDelete">{{ t('delete') }}</Button>
        </div>
        <div class="flex gap-2">
          <Button @click="handleClose">{{ t('cancel') }}</Button>
          <Button type="primary" @click="handleAdd">{{ isEdit ? t('save') : t('add') }}</Button>
        </div>
      </div>

      <!-- Delete confirmation footer -->
      <div v-else class="p-4 flex justify-end gap-2">
        <Button @click="deleteStep = 0">{{ t('cancel') }}</Button>
        <Button type="danger" @click="handleDelete">{{ deleteStep === 1 ? t('ok') : t('delete') }}</Button>
      </div>
    </template>
  </Dialog>
</template>

<script lang="ts" setup>
import { reactive, watch, computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import Button from '../base/Button.vue'
import Input from '../base/Input.vue'
import Dialog from '../base/Dialog.vue'

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

const emit = defineEmits(['close', 'add', 'delete'])

const endpointDraft = reactive({
  alias: '',
  url: '',
  apiKey: '',
})

const deleteStep = ref(0)

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
  } else {
    deleteStep.value = 0
  }
})

const handleClose = () => {
  endpointDraft.alias = ''
  endpointDraft.url = ''
  endpointDraft.apiKey = ''
  deleteStep.value = 0
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

const handleDelete = () => {
  if (deleteStep.value === 0) {
    deleteStep.value = 1
  } else if (deleteStep.value === 1) {
    deleteStep.value = 2
  } else {
    emit('delete', props.initialData?.id)
  }
}
</script>
