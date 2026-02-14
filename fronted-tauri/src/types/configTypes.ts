import {
    DEFAULT_LOAD_BALANCER_CONFIG,
    DEFAULT_PROXY_MODE,
    type LoadBalancerConfigV2,
    type ProxyMode,
} from './loadBalancerTypes'

export type ConverterType = 'codex' | 'gemini' | 'anthropic'

export interface EndpointOption {
    id: string
    alias: string
    url: string
    apiKey: string
    converter?: ConverterType
    codexModel?: string
    codexModelMapping?: CodexModelMapping
    codexEffortCapabilityMap?: CodexEffortCapabilityMap
    geminiModelPreset?: GeminiModelPreset
    reasoningEffort?: ReasoningEffort
    geminiReasoningEffort?: ReasoningEffort
    anthropicModelMapping?: AnthropicModelMapping
}

export interface ReasoningEffort {
    opus: string
    sonnet: string
    haiku: string
}

export interface CodexModelMapping {
    opus: string
    sonnet: string
    haiku: string
}

export interface AnthropicModelMapping {
    opus: string
    sonnet: string
    haiku: string
}

export type CodexEffortCapabilityMap = Record<string, string[]>
export type GeminiModelPreset = string[]

export interface ProxyConfig {
    port: number
    targetUrl: string
    apiKey: string
    endpointOptions: EndpointOption[]
    selectedEndpointId: string
    converter: ConverterType
    codexModel: string
    codexModelMapping: CodexModelMapping
    anthropicModelMapping: AnthropicModelMapping
    codexEffortCapabilityMap: CodexEffortCapabilityMap
    geminiModelPreset: GeminiModelPreset
    maxConcurrency: number
    ignoreProbeRequests: boolean
    allowCountTokensFallbackEstimate: boolean
    lbModelCooldownSeconds: number
    lbTransientBackoffSeconds: number
    reasoningEffort: ReasoningEffort
    geminiReasoningEffort: ReasoningEffort
    skillInjectionPrompt: string
    lang: string
    force: boolean
}

export interface ProxyConfigV2 extends ProxyConfig {
    proxyMode?: ProxyMode
    loadBalancer?: LoadBalancerConfigV2
}

export const DEFAULT_PROXY_CONFIG_V2: Pick<Required<ProxyConfigV2>, 'proxyMode' | 'loadBalancer'> = {
    proxyMode: DEFAULT_PROXY_MODE,
    loadBalancer: DEFAULT_LOAD_BALANCER_CONFIG,
}
