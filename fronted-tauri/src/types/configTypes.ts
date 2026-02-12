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

export type CodexEffortCapabilityMap = Record<string, string[]>
export type GeminiModelPreset = string[]

export type ConverterType = 'codex' | 'gemini'

export interface ProxyConfig {
    port: number
    targetUrl: string
    apiKey: string
    endpointOptions: EndpointOption[]
    selectedEndpointId: string
    converter: ConverterType
    codexModel: string
    codexModelMapping: CodexModelMapping
    codexEffortCapabilityMap: CodexEffortCapabilityMap
    geminiModelPreset: GeminiModelPreset
    maxConcurrency: number
    ignoreProbeRequests: boolean
    allowCountTokensFallbackEstimate: boolean
    reasoningEffort: ReasoningEffort
    geminiReasoningEffort: ReasoningEffort
    skillInjectionPrompt: string
    lang: string
    force: boolean
}
