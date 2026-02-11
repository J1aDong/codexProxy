<template>
  <div class="relative">
    <label v-if="label" class="block text-sm font-medium text-apple-text-primary mb-1">
      {{ label }}
    </label>
    <div class="relative">
      <button
        :class="[
          'w-full px-3 py-2.5 rounded-lg bg-gray-100 border border-transparent focus:bg-white focus:border-apple-blue focus:ring-2 focus:ring-apple-blue focus:ring-opacity-20 transition-all duration-200 outline-none text-left flex justify-between items-center',
          { 'opacity-50 cursor-not-allowed': disabled },
        ]"
        :disabled="disabled"
        @click="toggleDropdown"
      >
        <span class="block min-w-0 flex-1 pr-2 overflow-hidden">
          <span
            ref="labelRef"
            class="block w-full whitespace-nowrap leading-[1.2]"
          >
            {{ selectedOption?.label || placeholder }}
          </span>
        </span>
        <svg
          class="w-4 h-4 text-apple-text-secondary transition-transform duration-200 shrink-0"
          :class="{ 'transform rotate-180': isOpen }"
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7" />
        </svg>
      </button>
      <div
        v-if="isOpen"
        class="absolute z-50 w-full mt-1 bg-white rounded-lg shadow-lg border border-gray-200 max-h-60 overflow-y-auto"
        @click.outside="closeDropdown"
      >
        <div
          v-for="option in options"
          :key="option.value"
          class="px-3 py-2.5 cursor-pointer hover:bg-gray-50 text-sm transition-colors duration-150 flex items-center justify-between group"
          :class="{ 'text-apple-blue bg-blue-50': selectedOption?.value === option.value }"
          @click="selectOption(option)"
        >
          <slot name="option" :option="option">
            <span>{{ option.label }}</span>
          </slot>
        </div>
      </div>
    </div>
    <div v-if="tip" class="text-apple-text-secondary text-xs mt-1">
      {{ tip }}
    </div>
  </div>
</template>

<script lang="ts" setup>
import { ref, computed, nextTick, watch, onMounted, onUnmounted } from 'vue'
import fitty, { type FittyInstance } from 'fitty'

interface Option {
  value: string | number
  label: string
}

const props = defineProps({
  label: {
    type: String,
    default: '',
  },
  placeholder: {
    type: String,
    default: '',
  },
  modelValue: {
    type: [String, Number],
    default: '',
  },
  options: {
    type: Array as () => Option[],
    default: () => [],
  },
  disabled: {
    type: Boolean,
    default: false,
  },
  tip: {
    type: String,
    default: '',
  },
})

const emit = defineEmits(['update:modelValue', 'change'])

const isOpen = ref(false)
const labelRef = ref<HTMLSpanElement | null>(null)
let fittyInstance: FittyInstance | null = null

const selectedOption = computed(() => {
  return props.options.find(option => option.value === props.modelValue)
})

const toggleDropdown = () => {
  if (!props.disabled) {
    isOpen.value = !isOpen.value
  }
}

const closeDropdown = () => {
  isOpen.value = false
}

const selectOption = (option: Option) => {
  emit('update:modelValue', option.value)
  emit('change', option.value)
  isOpen.value = false
}

const fitLabelToWidth = async () => {
  await nextTick()
  fittyInstance?.fit({ sync: true })
}

watch(
  () => [props.modelValue, props.placeholder, props.options],
  () => {
    fitLabelToWidth().catch(() => {})
  },
  { deep: true }
)

onMounted(() => {
  if (labelRef.value) {
    fittyInstance = fitty(labelRef.value, {
      minSize: 11,
      maxSize: 16,
      multiLine: false,
      observeMutations: {
        subtree: true,
        childList: true,
        characterData: true,
      },
    })
  }

  fitLabelToWidth().catch(() => {})
})

onUnmounted(() => {
  if (fittyInstance) {
    fittyInstance.unsubscribe()
    fittyInstance = null
  }
})
</script>
