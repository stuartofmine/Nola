import type { ModelValidationIssue } from './modelValidation.ts';

export type RemoteProfile = {
    id: string; provider?: string; displayName: string; modelName: string; revision: number;
    enabled: boolean; selected: boolean; priority: number; order: number;
    endpoint: { id: string; name: string; baseURL: string; wireProtocol: string; enabled: boolean; order: number };
    capabilities: string[];
    executionCapabilities?: ModelExecutionCapabilities | null;
    executionCapabilityStatus?: 'needs_confirmation' | 'text_only' | 'tools_enabled';
    reasoning?: ReasoningConfiguration | null;
    parameters: Record<string, number | string | boolean | null>;
};

/** Unsaved values are never sent as a declaration without user confirmation. */
export type RemoteProfileDraft = RemoteProfile & { executionCapabilityDraft?: ModelExecutionCapabilities };

export type ModelExecutionCapabilities = {
    toolCalls: boolean;
    imageInput: boolean;
    reasoning: boolean;
    samplingParameters: boolean;
    samplingRequiresNoReasoning?: boolean;
    chatOutputTokenField?: 'auto' | 'max_tokens' | 'max_completion_tokens';
};

export const importExecutionCapabilities = (): ModelExecutionCapabilities => ({
    toolCalls: true, imageInput: false, reasoning: false, samplingParameters: false,
    samplingRequiresNoReasoning: false,
    chatOutputTokenField: 'auto',
});

export type ReasoningConfiguration = {
    control: 'automatic' | 'effort' | 'budget';
    efforts?: string[]; budgets?: number[];
    responses?: boolean; outputLimit?: number;
    defaultPreference?: string | null;
};

export function canSelectProfile(profile: RemoteProfile): boolean {
    return profile.enabled && profile.endpoint.enabled !== false && profile.capabilities.includes('chat');
}

const efforts = ['none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max'];

export function reasoningOptions(value: ReasoningConfiguration, protocol: string, outputLimit = value.outputLimit ?? 16384) {
    if (value.control === 'effort' && ['responses', 'chat_completions', 'anthropic'].includes(protocol)) {
        return efforts.filter(option => value.efforts?.includes(option));
    }
    return value.control === 'budget' && protocol === 'anthropic'
        && value.budgets?.length === 3
        && value.budgets.every((budget, index, budgets) => Number.isSafeInteger(budget) && budget >= 1024 && budget < outputLimit
            && (index === 0 || budget > budgets[index - 1]))
        ? ['concise', 'balanced', 'detailed'] : [];
}

export function reasoningValidationError(value: ReasoningConfiguration, protocol: string, outputLimit: number): ModelValidationIssue | null {
    if (!['responses', 'chat_completions', 'anthropic'].includes(protocol)) return { code: 'modelValidation.protocol' };
    if (value.efforts?.some(option => !efforts.includes(option))) return { code: 'modelValidation.efforts' };
    if (value.budgets?.some(budget => !Number.isSafeInteger(budget) || budget < 0)) return { code: 'modelValidation.budgets' };
    if (value.control === 'budget' && protocol !== 'anthropic') {
        return { code: 'modelValidation.control' };
    }
    const options = reasoningOptions(value, protocol, outputLimit);
    if (value.control === 'budget' && options.length === 0) return { code: 'modelValidation.budgetTiers', params: { minimum: 1024 } };
    if (value.control === 'effort' && options.length === 0) return { code: 'modelValidation.effortRequired' };
    if (value.defaultPreference != null && value.defaultPreference !== 'automatic' && !options.includes(value.defaultPreference)
        && !(protocol === 'chat_completions' && value.control === 'effort' && ['concise', 'balanced', 'detailed'].includes(value.defaultPreference))) {
        return { code: 'modelValidation.default' };
    }
    return null;
}

export function reasoningExecutionValidationError(value: ReasoningConfiguration | null | undefined,
    execution: ModelExecutionCapabilities | null | undefined): ModelValidationIssue | null {
    if (execution == null) return { code: 'modelValidation.executionDeclaration' };
    return value?.control !== undefined && value.control !== 'automatic' && !execution?.reasoning
        ? { code: 'modelValidation.executionReasoning' } : null;
}

export function executionParameterValidationError(profile: RemoteProfile): ModelValidationIssue | null {
    if (profile.executionCapabilities == null) return { code: 'modelValidation.executionDeclaration' };
    const parameters = profile.parameters;
    if (profile.executionCapabilities?.samplingRequiresNoReasoning
        && (!profile.executionCapabilities.samplingParameters || !profile.executionCapabilities.reasoning
            || profile.endpoint.wireProtocol !== 'responses' || profile.reasoning?.control !== 'effort'
            || !profile.reasoning.efforts?.includes('none'))) return { code: 'modelValidation.executionSamplingReasoning' };
    if (!profile.executionCapabilities?.samplingParameters
        && (Number(parameters.temperature ?? 0.7) !== 0.7 || Number(parameters.topP ?? 0.9) !== 0.9)) {
        return { code: 'modelValidation.executionSampling' };
    }
    const penalty = Number(parameters.presencePenalty ?? 0) !== 0 || Number(parameters.frequencyPenalty ?? 0) !== 0;
    const seed = parameters.seed != null;
    const stop = String(parameters.stopSequences ?? '') !== '';
    if (profile.endpoint.wireProtocol === 'responses' && (penalty || seed || stop)
        || profile.endpoint.wireProtocol === 'anthropic' && (penalty || seed)) {
        return { code: 'modelValidation.protocolParameters' };
    }
    return null;
}

export function metadataDraft(profile: RemoteProfileDraft) {
    return { displayName: profile.displayName, enabled: profile.enabled, priority: profile.priority ?? 50,
        capabilities: [...(profile.capabilities ?? [])],
        executionCapabilities: { ...(profile.executionCapabilities ?? profile.executionCapabilityDraft ?? importExecutionCapabilities()) },
        executionCapabilitiesConfirmed: profile.executionCapabilities != null };
}

export function metadataChanges(profile: RemoteProfile, draft: ReturnType<typeof metadataDraft>) {
    const original = metadataDraft(profile);
    const changes: Record<string, unknown> = Object.fromEntries((['displayName', 'enabled', 'priority', 'capabilities'] as const)
        .filter(key => key === 'capabilities'
            ? JSON.stringify([...original.capabilities].sort()) !== JSON.stringify([...draft.capabilities].sort())
            : original[key] !== draft[key])
        .map(key => [key, draft[key]]));
    if (draft.executionCapabilitiesConfirmed && (profile.executionCapabilities == null
        || JSON.stringify(original.executionCapabilities) !== JSON.stringify(draft.executionCapabilities))) {
        changes.executionCapabilities = { ...draft.executionCapabilities };
    }
    return changes;
}

export function reasoningDraft(value: ReasoningConfiguration | null | undefined): ReasoningConfiguration & { efforts: string[]; budgets: number[] } {
    return { ...value, control: value?.control ?? 'automatic', efforts: [...(value?.efforts ?? [])], budgets: [...(value?.budgets ?? [1024, 4096, 8192])] };
}

export function reasoningChanges(original: RemoteProfile['reasoning'], draft: ReasoningConfiguration) {
    return JSON.stringify(reasoningDraft(original)) === JSON.stringify(draft) ? {} : { reasoning: draft };
}

export function groupedProfiles(profiles: RemoteProfile[]) {
    const groups = new Map<string, { endpoint: RemoteProfile['endpoint']; models: RemoteProfile[] }>();
    for (const model of profiles) {
        const id = model.endpoint.id;
        if (!groups.has(id)) groups.set(id, { endpoint: model.endpoint, models: [] });
        groups.get(id)!.models.push(model);
    }
    return [...groups.values()].sort((a, b) => a.endpoint.order - b.endpoint.order || a.endpoint.id.localeCompare(b.endpoint.id))
        .map(group => ({ ...group, models: [...group.models].sort((a, b) => a.order - b.order || a.id.localeCompare(b.id)) }));
}

export function parameterChanges(original: RemoteProfile['parameters'], draft: RemoteProfile['parameters']) {
    const allowed = ['temperature', 'topP', 'maxOutputTokens', 'maxContextLength', 'presencePenalty', 'frequencyPenalty', 'seed', 'stopSequences'];
    return Object.fromEntries(allowed.filter(key => key in draft && draft[key] !== original[key]).map(key => [key, draft[key]]));
}
