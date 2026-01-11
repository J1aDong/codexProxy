import { contextBridge, ipcRenderer } from 'electron'

contextBridge.exposeInMainWorld('ipcRenderer', {
  send: (channel: string, data: any) => {
    ipcRenderer.send(channel, data)
  },
  on: (channel: string, func: (...args: any[]) => void) => {
    // Deliberately strip event as it includes `sender` 
    ipcRenderer.on(channel, (event, ...args) => func(event, ...args))
  },
  off: (channel: string, func: (...args: any[]) => void) => {
    ipcRenderer.removeListener(channel, func)
  },
  invoke: (channel: string, ...args: any[]) => ipcRenderer.invoke(channel, ...args)
})
