import { invoke } from '@tauri-apps/api/core'
import type { ProxyConfigV2 } from '../types/configTypes'

export const loadConfig = (): Promise<ProxyConfigV2 | null> =>
    invoke<ProxyConfigV2 | null>('load_config')

export const saveConfig = (config: ProxyConfigV2): Promise<void> =>
    invoke('save_config', { config })

export const startProxy = (config: ProxyConfigV2): Promise<void> =>
    invoke('start_proxy', { config })

export const applyProxyConfig = (config: ProxyConfigV2): Promise<void> =>
    invoke('apply_proxy_config', { config })

export const restartProxy = (config: ProxyConfigV2): Promise<void> =>
    invoke('restart_proxy', { config })

export const stopProxy = (): Promise<void> =>
    invoke('stop_proxy')

export const saveLang = (lang: string): Promise<void> =>
    invoke('save_lang', { lang })

export const exportConfig = (): Promise<string> =>
    invoke<string>('export_config')

export const importConfig = (configJson: string): Promise<void> =>
    invoke('import_config', { configJson })
