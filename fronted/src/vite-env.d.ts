/// <reference types="vite/client" />

declare module '*.vue' {
  import type { DefineComponent } from 'vue'
  const component: DefineComponent<{}, {}, any>
  export default component
}

interface Window {
  ipcRenderer: {
    send(channel: string, data?: any): void
    on(channel: string, func: (...args: any[]) => void): void
    off(channel: string, func: (...args: any[]) => void): void
    invoke(channel: string, ...args: any[]): Promise<any>
  }
}
