<template>
  <Dialog
    :visible="visible"
    :title="t('importExportTitle')"
    @close="handleClose"
  >
    <div class="space-y-4">
      <!-- Tab Switch -->
      <div class="flex gap-2 p-1 bg-gray-100 dark:bg-dark-secondary rounded-lg">
        <button
          :class="['flex-1 py-2 px-4 rounded-md text-sm font-medium transition-all', activeTab === 'export' ? 'bg-white dark:bg-dark-primary shadow-sm text-apple-blue dark:text-blue-400' : 'text-gray-600 dark:text-dark-text-secondary hover:text-gray-900 dark:hover:text-dark-text-primary']"
          @click="activeTab = 'export'"
        >
          {{ t('exportConfig') }}
        </button>
        <button
          :class="['flex-1 py-2 px-4 rounded-md text-sm font-medium transition-all', activeTab === 'import' ? 'bg-white dark:bg-dark-primary shadow-sm text-apple-blue dark:text-blue-400' : 'text-gray-600 dark:text-dark-text-secondary hover:text-gray-900 dark:hover:text-dark-text-primary']"
          @click="activeTab = 'import'"
        >
          {{ t('importConfig') }}
        </button>
      </div>

      <!-- Export Tab -->
      <div v-if="activeTab === 'export'" class="space-y-4">
        <div class="text-sm text-apple-text-secondary dark:text-dark-text-secondary">
          {{ t('exportDescription') }}
        </div>
        <textarea
          v-model="exportJson"
          rows="12"
          readonly
          class="w-full px-3 py-2.5 rounded-lg bg-gray-50 dark:bg-dark-secondary border border-gray-200 dark:border-dark-border text-xs font-mono resize-none focus:outline-none text-apple-text-primary dark:text-dark-text-primary"
        />
        <div class="flex gap-3">
          <Button type="primary" class="flex-1" @click="handleCopyToClipboard">
            {{ copied ? t('copied') : t('copyToClipboard') }}
          </Button>
          <Button type="secondary" class="flex-1" @click="handleSaveToFile">
            {{ t('saveToFile') }}
          </Button>
        </div>
      </div>

      <!-- Import Tab -->
      <div v-if="activeTab === 'import'" class="space-y-4">
        <div class="text-sm text-apple-text-secondary dark:text-dark-text-secondary">
          {{ t('importDescription') }}
        </div>
        <textarea
          v-model="importJson"
          rows="10"
          :placeholder="t('importPlaceholder')"
          class="w-full px-3 py-2.5 rounded-lg bg-gray-100 dark:bg-dark-secondary border border-transparent dark:border-dark-border focus:bg-white dark:focus:bg-dark-primary focus:border-apple-blue dark:focus:border-blue-400 focus:ring-2 focus:ring-apple-blue focus:ring-opacity-20 dark:focus:ring-blue-400 dark:focus:ring-opacity-20 transition-all duration-200 outline-none resize-none text-xs font-mono text-apple-text-primary dark:text-dark-text-primary placeholder-gray-500 dark:placeholder-dark-text-secondary"
        />
        <div class="flex gap-3">
          <Button type="secondary" class="flex-1" @click="handleLoadFromFile">
            {{ t('loadFromFile') }}
          </Button>
          <Button type="primary" class="flex-1" :disabled="!importJson.trim()" @click="handleImport">
            {{ t('import') }}
          </Button>
        </div>
        <div v-if="importError" class="text-sm text-red-500 dark:text-red-400">
          {{ importError }}
        </div>
        <div v-if="importSuccess" class="text-sm text-green-600 dark:text-green-400">
          {{ t('importSuccess') }}
        </div>
      </div>
    </div>
  </Dialog>
</template>

<script lang="ts" setup>
import { ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { exportConfig, importConfig } from '../../bridge/configBridge'
import Button from '../base/Button.vue'
import Dialog from '../base/Dialog.vue'

const { t } = useI18n()

const props = defineProps({
  visible: {
    type: Boolean,
    required: true,
  },
})

const emit = defineEmits(['close', 'imported'])

const activeTab = ref<'export' | 'import'>('export')
const exportJson = ref('')
const importJson = ref('')
const importError = ref('')
const importSuccess = ref(false)
const copied = ref(false)

const loadExportData = async () => {
  try {
    const data = await exportConfig()
    exportJson.value = data
  } catch (error) {
    exportJson.value = JSON.stringify({ error: String(error) }, null, 2)
  }
}

watch(() => props.visible, (val) => {
  if (val) {
    activeTab.value = 'export'
    importJson.value = ''
    importError.value = ''
    importSuccess.value = false
    loadExportData()
  }
})

const handleClose = () => {
  emit('close')
}

const handleCopyToClipboard = async () => {
  try {
    await navigator.clipboard.writeText(exportJson.value)
    copied.value = true
    setTimeout(() => {
      copied.value = false
    }, 2000)
  } catch (error) {
    console.error('Failed to copy:', error)
  }
}

const handleSaveToFile = async () => {
  try {
    console.log('开始保存文件...')
    const { save } = await import('@tauri-apps/plugin-dialog')
    const { writeTextFile } = await import('@tauri-apps/plugin-fs')

    console.log('导入插件成功，准备打开保存对话框...')
    const filePath = await save({
      defaultPath: `codex-proxy-config-${new Date().toISOString().split('T')[0]}.json`,
      filters: [{
        name: 'JSON',
        extensions: ['json']
      }]
    })

    console.log('用户选择的文件路径:', filePath)
    if (filePath) {
      console.log('开始写入文件...')
      await writeTextFile(filePath, exportJson.value)
      console.log('文件保存成功!')
      // 添加成功提示
      alert('文件保存成功!')
    } else {
      console.log('用户取消了保存操作')
    }
  } catch (error) {
    console.error('保存文件失败:', error)
    alert(`保存文件失败: ${error}`)
  }
}

const handleLoadFromFile = async () => {
  try {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const { readTextFile } = await import('@tauri-apps/plugin-fs')

    const filePath = await open({
      filters: [{
        name: 'JSON',
        extensions: ['json']
      }]
    })

    if (filePath && typeof filePath === 'string') {
      const content = await readTextFile(filePath)
      importJson.value = content
      importError.value = ''
      importSuccess.value = false
    }
  } catch (error) {
    console.error('Failed to load file:', error)
    importError.value = 'Failed to load file'
  }
}

const handleImport = async () => {
  importError.value = ''
  importSuccess.value = false

  try {
    await importConfig(importJson.value.trim())
    importSuccess.value = true
    emit('imported')
    setTimeout(() => {
      handleClose()
    }, 1500)
  } catch (error) {
    importError.value = String(error)
  }
}
</script>
