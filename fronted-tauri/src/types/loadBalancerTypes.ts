export type LbConverterType = 'codex' | 'gemini' | 'anthropic'
export type ProxyMode = 'single' | 'load_balancer'

export interface LbSlotEndpointRef {
  endpointId: string
  customModelName?: string
  customReasoningEffort?: string
  converterOverride?: LbConverterType
}

export interface ModelSlotMapping {
  opus: LbSlotEndpointRef[]
  sonnet: LbSlotEndpointRef[]
  haiku: LbSlotEndpointRef[]
}

export interface LbFailoverStrategy {
  errorThreshold: number
  errorWindowSeconds: number
  cooldownSeconds: number
  degradedConcurrency: number
}

export interface LbEndpointConfig {
  endpointId: string
  enabled: boolean
  maxConcurrency: number
  priority: number
  weight: number
}

export interface LoadBalancerProfile {
  id: string
  name: string
  description?: string
  modelMapping: ModelSlotMapping
  strategy: LbFailoverStrategy
}

export interface LoadBalancerConfigV2 {
  lbProfiles: LoadBalancerProfile[]
  selectedLbProfileId?: string
  lbEndpointConfigs: Record<string, LbEndpointConfig>
}

export const DEFAULT_PROXY_MODE: ProxyMode = 'single'

export const DEFAULT_LB_FAILOVER_STRATEGY: LbFailoverStrategy = {
  errorThreshold: 5,
  errorWindowSeconds: 60,
  cooldownSeconds: 3600,
  degradedConcurrency: 4,
}

export const DEFAULT_LOAD_BALANCER_CONFIG: LoadBalancerConfigV2 = {
  lbProfiles: [],
  selectedLbProfileId: undefined,
  lbEndpointConfigs: {},
}
