<template>
  <div class="relative">
    <label v-if="label" class="block text-sm font-medium mb-1 text-apple-text-primary dark:text-dark-text-primary">
      {{ label }}
    </label>
    <div class="relative group">
      <input
        v-if="type !== 'textarea'"
        :type="inputType"
        :class="[
          'w-full px-3 py-2.5 rounded-lg bg-gray-100 border border-transparent focus:bg-white focus:border-apple-blue focus:ring-2 focus:ring-apple-blue focus:ring-opacity-20 transition-all duration-200 outline-none dark:bg-dark-tertiary dark:border-dark-border dark:text-dark-text-primary dark:focus:bg-dark-tertiary dark:focus:border-accent-blue',
          { 'border-red-500 focus:border-red-500 focus:ring-red-500': error },
          { 'pr-10': type === 'password' }
        ]"
        :placeholder="placeholder"
        :value="modelValue"
        :disabled="disabled"
        @input="handleInput"
        @blur="handleBlur"
      />
      <button
        v-if="type === 'password'"
        type="button"
        class="absolute right-3 top-1/2 -translate-y-1/2 text-gray-400 hover:text-apple-blue transition-colors duration-200 focus:outline-none dark:text-dark-text-tertiary dark:hover:text-accent-blue"
        @click="togglePassword"
      >
        <svg v-if="showPassword" class="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z" />
        </svg>
        <svg v-else class="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.542-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M9.882 9.882L5.13 5.13m13.74 13.74L13.882 13.88m0 0l4.99-4.99m-4.99 4.99c1.478 1.478 3.515 2.515 5.258 3.238L21.542 12c-1.274-4.057-5.064-7-9.542-7-1.18 0-2.302.203-3.344.575M3 3l18 18" />
        </svg>
      </button>

      <textarea
        v-else-if="type === 'textarea'"
        :rows="rows || 4"
        :class="[
          'w-full px-3 py-2.5 rounded-lg bg-gray-100 border border-transparent focus:bg-white focus:border-apple-blue focus:ring-2 focus:ring-apple-blue focus:ring-opacity-20 transition-all duration-200 outline-none resize-none dark:bg-dark-tertiary dark:border-dark-border dark:text-dark-text-primary dark:focus:bg-dark-tertiary dark:focus:border-accent-blue',
          { 'border-red-500 focus:border-red-500 focus:ring-red-500': error },
        ]"
        :placeholder="placeholder"
        :value="modelValue"
        :disabled="disabled"
        @input="handleInput"
        @blur="handleBlur"
      />
    </div>
    <div v-if="error" class="text-red-500 text-xs mt-1">
      {{ error }}
    </div>
    <div v-if="tip" class="text-apple-text-secondary dark:text-dark-text-secondary text-xs mt-1">
      {{ tip }}
    </div>
  </div>
</template>

<script lang="ts" setup>
import { ref, computed } from 'vue'

const props = defineProps({
  type: {
    type: String,
    default: 'text',
  },
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
  disabled: {
    type: Boolean,
    default: false,
  },
  error: {
    type: String,
    default: '',
  },
  tip: {
    type: String,
    default: '',
  },
  rows: {
    type: Number,
    default: 4,
  },
})

const emit = defineEmits(['update:modelValue', 'blur'])

const showPassword = ref(false)

const inputType = computed(() => {
  if (props.type === 'password') {
    return showPassword.value ? 'text' : 'password'
  }
  return props.type
})

const togglePassword = () => {
  showPassword.value = !showPassword.value
}

const handleInput = (event: Event) => {
  const target = event.target as HTMLInputElement | HTMLTextAreaElement
  emit('update:modelValue', target.value)
}

const handleBlur = () => {
  emit('blur')
}
</script>
