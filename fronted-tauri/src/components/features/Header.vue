<template>
  <div class="flex items-center justify-between mb-8">
    <div class="flex items-center gap-3">
      <StatusBadge
        :is-running="isRunning"
        :text="isRunning ? t('statusRunning') : t('statusStopped')"
      />
    </div>
    <h1 class="text-2xl font-semibold text-apple-text-primary font-pixel">
      {{ t('title') }}
    </h1>
    <div class="flex items-center gap-2">
      <!-- Menu Button -->
      <div class="header-menu-wrapper" ref="menuRef">
        <Button
          type="text"
          circle
          size="small"
          class="text-apple-blue hover:bg-blue-50"
          @click="toggleMenu"
        >
          <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <circle cx="12" cy="5" r="1.5" fill="currentColor" />
            <circle cx="12" cy="12" r="1.5" fill="currentColor" />
            <circle cx="12" cy="19" r="1.5" fill="currentColor" />
          </svg>
        </Button>

        <!-- Dropdown Menu -->
        <Transition name="menu-fade">
          <div v-if="menuVisible" class="header-dropdown">
            <div class="header-dropdown-item" @click="handleMenuItem('settings')">
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
              </svg>
              <span>{{ t('menuPromptSettings') }}</span>
            </div>
            <div class="header-dropdown-item" @click="handleMenuItem('lang')">
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M3 5h12M9 3v2m1.048 9.5A18.022 18.022 0 016.412 9m6.088 9h7M11 21l5-10 5 10M12.751 5C11.783 10.77 8.07 15.61 3 18.129" />
              </svg>
              <span>{{ locale === 'zh' ? 'English' : '中文' }}</span>
            </div>
            <div class="header-dropdown-item" @click="handleMenuItem('advancedSettings')">
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
              </svg>
              <span>{{ t('menuAdvancedSettings') }}</span>
            </div>
            <div class="header-dropdown-item" @click="handleMenuItem('importExport')">
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M8 7h12m0 0l-4-4m4 4l-4 4m0 6H4m0 0l4 4m-4-4l4-4" />
              </svg>
              <span>{{ t('menuImportExport') }}</span>
            </div>
            <div class="header-dropdown-divider"></div>
            <div class="header-dropdown-item" @click="handleMenuItem('about')">
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
              </svg>
              <span>{{ t('menuAbout') }}</span>
            </div>
          </div>
        </Transition>
      </div>

      <!-- Logs Button -->
      <Button
        type="text"
        circle
        size="small"
        class="text-apple-blue hover:bg-blue-50"
        @click="handleShowLogs"
      >
        <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" />
        </svg>
      </Button>
    </div>
  </div>
</template>

<script lang="ts" setup>
import { ref, onMounted, onUnmounted } from 'vue'
import { useI18n } from 'vue-i18n'
import Button from '../base/Button.vue'
import StatusBadge from './StatusBadge.vue'

const { t, locale } = useI18n()

defineProps({
  isRunning: {
    type: Boolean,
    required: true,
  },
})

const emit = defineEmits(['toggleLang', 'showAbout', 'showSettings', 'showAdvancedSettings', 'showImportExport', 'showLogs'])

const menuVisible = ref(false)
const menuRef = ref<HTMLElement | null>(null)

const toggleMenu = () => {
  menuVisible.value = !menuVisible.value
}

const closeMenu = () => {
  menuVisible.value = false
}

const handleMenuItem = (action: string) => {
  closeMenu()
  switch (action) {
    case 'settings':
      emit('showSettings')
      break
    case 'lang':
      emit('toggleLang')
      break
    case 'advancedSettings':
      emit('showAdvancedSettings')
      break
    case 'importExport':
      emit('showImportExport')
      break
    case 'about':
      emit('showAbout')
      break
  }
}

const handleShowLogs = () => {
  emit('showLogs')
}

const onClickOutside = (e: MouseEvent) => {
  if (menuRef.value && !menuRef.value.contains(e.target as Node)) {
    closeMenu()
  }
}

onMounted(() => {
  document.addEventListener('click', onClickOutside)
})

onUnmounted(() => {
  document.removeEventListener('click', onClickOutside)
})
</script>

<style scoped>
.font-pixel {
  font-family: "DotGothic16", sans-serif;
  line-height: 1.5;
  font-size: 1.25rem;
  padding-top: 4px;
}

.header-menu-wrapper {
  position: relative;
}

.header-dropdown {
  position: absolute;
  top: calc(100% + 6px);
  right: 0;
  min-width: 180px;
  background: white;
  border-radius: 12px;
  box-shadow: 0 4px 24px rgba(0, 0, 0, 0.12), 0 0 0 1px rgba(0, 0, 0, 0.04);
  padding: 6px;
  z-index: 100;
}

.header-dropdown-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 12px;
  border-radius: 8px;
  font-size: 13px;
  color: #1d1d1f;
  cursor: pointer;
  transition: background-color 0.15s ease;
  user-select: none;
}

.header-dropdown-item:hover {
  background-color: #f0f0f5;
}

.header-dropdown-item:active {
  background-color: #e5e5ea;
}

.header-dropdown-divider {
  height: 1px;
  background-color: #e5e5ea;
  margin: 4px 8px;
}

.menu-fade-enter-active,
.menu-fade-leave-active {
  transition: opacity 0.15s ease, transform 0.15s ease;
}

.menu-fade-enter-from,
.menu-fade-leave-to {
  opacity: 0;
  transform: translateY(-4px) scale(0.97);
}
</style>
