export interface EndpointOption {
    id: string
    alias: string
    url: string
    apiKey: string
}

export interface ReasoningEffort {
    opus: string
    sonnet: string
    haiku: string
}

export interface ProxyConfig {
    port: number
    targetUrl: string
    apiKey: string
    endpointOptions: EndpointOption[]
    selectedEndpointId: string
    codexModel: string
    maxConcurrency: number
    reasoningEffort: ReasoningEffort
    skillInjectionPrompt: string
    lang: string
    force: boolean
}
