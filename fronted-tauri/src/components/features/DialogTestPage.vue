<template>
  <div class="p-8 space-y-4">
    <h1 class="text-2xl font-bold mb-6 text-apple-text-primary dark:text-dark-text-primary">Dialog 组件测试</h1>

    <div class="space-y-4">
      <Button @click="showShortDialog = true">显示短内容 Dialog</Button>
      <Button @click="showLongDialog = true">显示长内容 Dialog</Button>
      <Button @click="showVeryLongDialog = true">显示超长内容 Dialog</Button>
    </div>

    <!-- 短内容 Dialog -->
    <Dialog
      :visible="showShortDialog"
      title="短内容测试"
      @close="showShortDialog = false"
    >
      <div class="space-y-4">
        <p class="text-apple-text-primary dark:text-dark-text-primary">这是一个短内容的 Dialog，应该显示为内容的自然高度。</p>
        <p class="text-apple-text-primary dark:text-dark-text-primary">内容不多，不会触发滚动。</p>
      </div>

      <template #footer>
        <div class="p-4 flex justify-end">
          <Button type="primary" @click="showShortDialog = false">确定</Button>
        </div>
      </template>
    </Dialog>

    <!-- 长内容 Dialog -->
    <Dialog
      :visible="showLongDialog"
      title="长内容测试"
      @close="showLongDialog = false"
    >
      <div class="space-y-4">
        <p class="text-apple-text-primary dark:text-dark-text-primary">这是一个长内容的 Dialog 测试。</p>
        <div v-for="i in 15" :key="i" class="p-4 bg-gray-100 dark:bg-dark-secondary rounded">
          <h3 class="font-semibold text-apple-text-primary dark:text-dark-text-primary">段落 {{ i }}</h3>
          <p class="text-apple-text-primary dark:text-dark-text-primary">这是第 {{ i }} 个段落的内容。当内容超过视口高度的 75% 时，Dialog 会自动启用滚动功能，确保用户可以查看所有内容而不会超出屏幕范围。</p>
        </div>
      </div>

      <template #footer>
        <div class="p-4 flex justify-end">
          <Button type="primary" @click="showLongDialog = false">确定</Button>
        </div>
      </template>
    </Dialog>

    <!-- 超长内容 Dialog -->
    <Dialog
      :visible="showVeryLongDialog"
      title="超长内容测试"
      @close="showVeryLongDialog = false"
    >
      <div class="space-y-4">
        <p class="text-apple-text-primary dark:text-dark-text-primary">这是一个超长内容的 Dialog 测试，用于验证滚动功能。</p>
        <div v-for="i in 30" :key="i" class="p-4 bg-gray-100 dark:bg-dark-secondary rounded">
          <h3 class="font-semibold text-apple-text-primary dark:text-dark-text-primary">段落 {{ i }}</h3>
          <p class="text-apple-text-primary dark:text-dark-text-primary">这是第 {{ i }} 个段落的内容。这个 Dialog 包含大量内容，肯定会超过视口高度的 75%，因此会启用滚动功能。用户可以通过滚动来查看所有内容。</p>
          <ul class="mt-2 space-y-1 text-apple-text-primary dark:text-dark-text-primary">
            <li>• 列表项 1</li>
            <li>• 列表项 2</li>
            <li>• 列表项 3</li>
          </ul>
        </div>
      </div>

      <template #footer>
        <div class="p-4 flex justify-end space-x-2">
          <Button @click="showVeryLongDialog = false">取消</Button>
          <Button type="primary" @click="showVeryLongDialog = false">确定</Button>
        </div>
      </template>
    </Dialog>
  </div>
</template>

<script lang="ts" setup>
import { ref } from 'vue'
import Button from '../base/Button.vue'
import Dialog from '../base/Dialog.vue'

const showShortDialog = ref(false)
const showLongDialog = ref(false)
const showVeryLongDialog = ref(false)
</script>