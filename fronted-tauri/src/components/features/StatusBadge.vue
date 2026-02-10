<template>
  <div
    class="flex items-center px-3 py-1.5 rounded-full text-xs font-medium transition-all duration-300 relative overflow-hidden"
    :class="{
      'bg-gray-200 text-apple-text-secondary': !isRunning,
      'bg-green-100/80 text-green-700': isRunning,
    }"
  >
    <!-- Dot and Ripple -->
    <div class="relative w-2 h-2 mr-2 flex items-center justify-center">
      <!-- Outer Ripple Layers (Only when running) -->
      <template v-if="isRunning">
        <div class="absolute w-full h-full rounded-full bg-green-500 animate-ripple opacity-30"></div>
        <div class="absolute w-full h-full rounded-full bg-green-500 animate-ripple-delayed opacity-20"></div>
      </template>
      <!-- Core Dot -->
      <div 
        class="relative w-2 h-2 rounded-full bg-current transition-transform duration-300"
        :class="{ 'scale-110': isRunning }"
      ></div>
    </div>

    <!-- Animated Characters -->
    <div class="status-text flex" ref="textRef">
      <span
        v-for="(char, index) in characters"
        :key="index"
        class="char inline-block whitespace-pre"
      >
        {{ char }}
      </span>
    </div>
  </div>
</template>

<script lang="ts" setup>
import { computed, ref, onMounted, watch, nextTick } from 'vue'
import { gsap } from 'gsap'

const props = defineProps({
  isRunning: {
    type: Boolean,
    required: true,
  },
  text: {
    type: String,
    required: true,
  },
})

const textRef = ref<HTMLElement | null>(null)
const characters = computed(() => props.text.split(''))

// GSAP Animation Logic
let animation: gsap.core.Timeline | null = null

const playAnimation = () => {
  if (!textRef.value) return
  
  // Kill previous animation if any
  if (animation) {
    animation.kill()
  }

  const chars = textRef.value.querySelectorAll('.char')
  if (!chars.length) return

  if (props.isRunning) {
    // Excited jumpy animation for running state
    animation = gsap.timeline({ repeat: -1 })
    animation.to(chars, {
      y: -3,
      duration: 0.4,
      ease: 'power1.inOut',
      stagger: {
        each: 0.08,
        from: 'start',
      },
      yoyo: true,
      repeat: 1,
    })
  } else {
    // Reset or discrete pulse for idle state
    animation = gsap.timeline()
    gsap.set(chars, { y: 0, opacity: 0.7 })
    animation.to(chars, {
      opacity: 1,
      duration: 0.8,
      stagger: 0.05,
      ease: 'none',
    })
  }
}

// Watch for running state change
watch(() => props.isRunning, async () => {
  await nextTick()
  playAnimation()
})

// Watch for text change (language toggle)
watch(() => props.text, async () => {
  await nextTick()
  playAnimation()
})

onMounted(() => {
  playAnimation()
})
</script>

<style scoped>
@keyframes ripple {
  0% {
    transform: scale(1);
    opacity: 0.3;
  }
  100% {
    transform: scale(3.5);
    opacity: 0;
  }
}

.animate-ripple {
  animation: ripple 2s cubic-bezier(0.4, 0, 0.2, 1) infinite;
}

.animate-ripple-delayed {
  animation: ripple 2s cubic-bezier(0.4, 0, 0.2, 1) infinite;
  animation-delay: 1s;
}

.char {
  display: inline-block;
  transform-origin: center bottom;
}
</style>
