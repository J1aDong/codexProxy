<template>
  <div class="flex items-center justify-between mb-8">
    <div class="flex items-center gap-3">
      <div
        class="flex items-center px-3 py-1.5 rounded-full text-xs font-medium transition-all duration-300"
        :class="{
          'bg-gray-200 text-apple-text-secondary': !isRunning,
          'bg-green-100 text-green-700': isRunning,
        }"
      >
        <div class="w-2 h-2 rounded-full bg-current mr-2"></div>
        {{ isRunning ? t.statusRunning : t.statusStopped }}
      </div>
    </div>
    <h1 class="text-2xl font-semibold text-apple-text-primary font-pixel">
      {{ t.title }}
    </h1>
    <div class="flex items-center gap-2">
      <Button
        type="text"
        circle
        size="small"
        class="text-apple-blue hover:bg-blue-50"
        @click="handleShowAbout"
      >
        <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
        </svg>
      </Button>
      <Button
        type="text"
        size="small"
        class="text-apple-blue hover:bg-blue-50 px-2"
        @click="toggleLang"
      >
        {{ lang === 'zh' ? 'En' : '中' }}
      </Button>
      <Button
        type="text"
        circle
        size="small"
        class="text-apple-blue hover:bg-blue-50"
        @click="handleShowSettings"
      >
        <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
        </svg>
      </Button>
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
import Button from '../base/Button.vue'

defineProps({
  isRunning: {
    type: Boolean,
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

const emit = defineEmits(['toggleLang', 'showAbout', 'showSettings', 'showLogs'])

const toggleLang = () => {
  emit('toggleLang')
}

const handleShowAbout = () => {
  emit('showAbout')
}

const handleShowSettings = () => {
  emit('showSettings')
}

const handleShowLogs = () => {
  emit('showLogs')
}
</script>

<style scoped>
.font-pixel {
  font-family: "DotGothic16", sans-serif;
  line-height: 1.5;
  font-size: 1.25rem;
  padding-top: 4px;
}
</style>
