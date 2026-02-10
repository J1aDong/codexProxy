<template>
  <div class="relative">
    <label v-if="label" class="block text-sm font-medium text-apple-text-primary mb-1">
      {{ label }}
    </label>
    <input
      v-if="type !== 'textarea'"
      :type="type"
      :class="[
        'w-full px-3 py-2.5 rounded-lg bg-gray-100 border border-transparent focus:bg-white focus:border-apple-blue focus:ring-2 focus:ring-apple-blue focus:ring-opacity-20 transition-all duration-200 outline-none',
        { 'border-red-500 focus:border-red-500 focus:ring-red-500': error },
      ]"
      :placeholder="placeholder"
      :value="modelValue"
      :disabled="disabled"
      @input="handleInput"
      @blur="handleBlur"
    />
    <textarea
      v-else
      :rows="rows || 4"
      :class="[
        'w-full px-3 py-2.5 rounded-lg bg-gray-100 border border-transparent focus:bg-white focus:border-apple-blue focus:ring-2 focus:ring-apple-blue focus:ring-opacity-20 transition-all duration-200 outline-none resize-none',
        { 'border-red-500 focus:border-red-500 focus:ring-red-500': error },
      ]"
      :placeholder="placeholder"
      :value="modelValue"
      :disabled="disabled"
      @input="handleInput"
      @blur="handleBlur"
    />
    <div v-if="error" class="text-red-500 text-xs mt-1">
      {{ error }}
    </div>
    <div v-if="tip" class="text-apple-text-secondary text-xs mt-1">
      {{ tip }}
    </div>
  </div>
</template>

<script lang="ts" setup>
import { } from 'vue'

defineProps({
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

const handleInput = (event: Event) => {
  const target = event.target as HTMLInputElement | HTMLTextAreaElement
  emit('update:modelValue', target.value)
}

const handleBlur = () => {
  emit('blur')
}
</script>
