import { invoke } from '@tauri-apps/api/core'
import type { ProxyConfig } from '../types/configTypes'

export const loadConfig = (): Promise<ProxyConfig | null> =>
    invoke<ProxyConfig | null>('load_config')

export const saveConfig = (config: ProxyConfig): Promise<void> =>
    invoke('save_config', { config })

export const startProxy = (config: ProxyConfig): Promise<void> =>
    invoke('start_proxy', { config })

export const stopProxy = (): Promise<void> =>
    invoke('stop_proxy')

export const saveLang = (lang: string): Promise<void> =>
    invoke('save_lang', { lang })

export const exportConfig = (): Promise<string> =>
    invoke<string>('export_config')

export const importConfig = (configJson: string): Promise<void> =>
    invoke('import_config', { configJson })
