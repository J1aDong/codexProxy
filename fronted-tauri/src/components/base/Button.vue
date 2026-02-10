<template>
  <button
    :class="[
      'font-medium transition-all duration-200 focus:outline-none focus:ring-2 focus:ring-offset-2',
      {
        'px-4 py-2': !circle, 
        'rounded-lg': !circle,
        'rounded-full': circle,
        'bg-apple-blue text-white hover:bg-blue-600 focus:ring-blue-500': type === 'primary',
        'bg-gray-100 text-apple-text-primary hover:bg-gray-200 focus:ring-gray-500': type === 'default',
        'bg-apple-danger text-white hover:bg-red-600 focus:ring-red-500': type === 'danger',
        'bg-transparent text-apple-text-secondary hover:text-apple-text-primary hover:bg-gray-100 focus:ring-gray-500': type === 'text',
        'bg-transparent text-apple-blue hover:bg-blue-50 focus:ring-blue-500': type === 'link',
        'flex items-center justify-center': circle,
        'w-8 h-8 p-0': circle && size === 'small',
        'w-10 h-10 p-0': circle && size === 'default',
        'w-12 h-12 p-0 text-lg': circle && size === 'large',
        'text-sm': size === 'small' && !circle,
        'text-base': size === 'default' && !circle,
        'text-lg': size === 'large' && !circle,
        'opacity-50 cursor-not-allowed': disabled,
      },
    ]"
    :disabled="disabled"
    @click="handleClick"
  >
    <slot>
      {{ label }}
    </slot>
  </button>
</template>

<script lang="ts" setup>
import { } from 'vue'

const props = defineProps({
  type: {
    type: String,
    default: 'default',
    validator: (value: string) => ['primary', 'default', 'danger', 'text', 'link'].includes(value),
  },
  size: {
    type: String,
    default: 'default',
    validator: (value: string) => ['small', 'default', 'large'].includes(value),
  },
  label: {
    type: String,
    default: '',
  },
  circle: {
    type: Boolean,
    default: false,
  },
  disabled: {
    type: Boolean,
    default: false,
  },
})

const emit = defineEmits(['click'])

const handleClick = () => {
  if (!props.disabled) {
    emit('click')
  }
}
</script>
