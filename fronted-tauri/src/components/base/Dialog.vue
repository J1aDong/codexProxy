<template>
  <div v-if="visible" class="fixed inset-0 bg-black bg-opacity-50 dark:bg-black dark:bg-opacity-70 flex items-center justify-center z-50 p-4">
    <div
      ref="dialogRef"
      class="bg-white dark:bg-dark-secondary rounded-xl w-full mx-4 flex flex-col"
      :class="[
        maxWidth,
        { 'max-h-[75vh]': shouldUseMaxHeight }
      ]"
      :style="dynamicStyles"
    >
      <!-- Header -->
      <div class="flex items-center justify-between p-4 border-b border-gray-200 dark:border-dark-border flex-shrink-0">
        <h2 class="text-lg font-semibold text-apple-text-primary dark:text-dark-text-primary">{{ title }}</h2>
        <Button
          v-if="showClose"
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

      <!-- Content -->
      <div
        ref="contentRef"
        class="flex-1 min-h-0"
        :class="{
          'overflow-y-auto': shouldUseMaxHeight,
          'p-4': !customPadding
        }"
      >
        <slot />
      </div>

      <!-- Footer -->
      <div v-if="$slots.footer" class="border-t border-gray-200 dark:border-dark-border flex-shrink-0">
        <slot name="footer" />
      </div>
    </div>
  </div>
</template>

<script lang="ts" setup>
import { ref, nextTick, watch, onMounted, onUnmounted } from 'vue'
import Button from './Button.vue'

const props = defineProps({
  visible: {
    type: Boolean,
    required: true,
  },
  title: {
    type: String,
    required: true,
  },
  showClose: {
    type: Boolean,
    default: true,
  },
  maxWidth: {
    type: String,
    default: 'max-w-md',
  },
  customPadding: {
    type: Boolean,
    default: false,
  },
})

const emit = defineEmits(['close'])

const dialogRef = ref<HTMLElement>()
const contentRef = ref<HTMLElement>()
const shouldUseMaxHeight = ref(false)
const dynamicStyles = ref({})

const handleClose = () => {
  emit('close')
}

const checkContentHeight = async () => {
  if (!dialogRef.value || !contentRef.value) return

  await nextTick()

  // 获取视口高度
  const viewportHeight = window.innerHeight
  const maxDialogHeight = viewportHeight * 0.75 // 75% of viewport height

  // 临时移除最大高度限制来测量内容的自然高度
  shouldUseMaxHeight.value = false
  dynamicStyles.value = {}

  await nextTick()

  // 获取 dialog 的自然高度
  const dialogNaturalHeight = dialogRef.value.scrollHeight

  if (dialogNaturalHeight > maxDialogHeight) {
    // 如果内容高度超过75%，启用滚动
    shouldUseMaxHeight.value = true
    dynamicStyles.value = {
      maxHeight: `${maxDialogHeight}px`
    }
  } else {
    // 否则使用自然高度
    shouldUseMaxHeight.value = false
    dynamicStyles.value = {}
  }
}

// 监听 visible 变化，重新计算高度
watch(() => props.visible, async (newVisible) => {
  if (newVisible) {
    await nextTick()
    checkContentHeight()
  }
})

// 监听窗口大小变化
const handleResize = () => {
  if (props.visible) {
    checkContentHeight()
  }
}

onMounted(() => {
  window.addEventListener('resize', handleResize)
})

onUnmounted(() => {
  window.removeEventListener('resize', handleResize)
})

// 暴露方法供外部调用
defineExpose({
  checkContentHeight
})
</script>