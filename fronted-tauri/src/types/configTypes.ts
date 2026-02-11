export interface EndpointOption {
    id: string
    alias: string
    url: string
    apiKey: string
    converter?: ConverterType
    codexModel?: string
    reasoningEffort?: ReasoningEffort
    geminiReasoningEffort?: ReasoningEffort
}

export interface ReasoningEffort {
    opus: string
    sonnet: string
    haiku: string
}

export type ConverterType = 'codex' | 'gemini'

export interface ProxyConfig {
    port: number
    targetUrl: string
    apiKey: string
    endpointOptions: EndpointOption[]
    selectedEndpointId: string
    converter: ConverterType
    codexModel: string
    maxConcurrency: number
    reasoningEffort: ReasoningEffort
    geminiReasoningEffort: ReasoningEffort
    skillInjectionPrompt: string
    lang: string
    force: boolean
}
