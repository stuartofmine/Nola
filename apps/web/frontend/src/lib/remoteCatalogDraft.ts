import type { RemoteProfile, RemoteProfileDraft } from './modelProfiles.ts';
import { executionParameterValidationError, groupedProfiles, reasoningExecutionValidationError } from './modelProfiles.ts';
import { endpointSnapshot, endpointValidationError } from './modelEndpoints.ts';
import { ModelValidationError, type ModelValidationIssue } from './modelValidation.ts';

const editableParameters = ['temperature', 'topP', 'maxOutputTokens', 'maxContextLength',
    'presencePenalty', 'frequencyPenalty', 'seed', 'stopSequences'];

export function copyRemoteCatalog(profiles: RemoteProfileDraft[]): RemoteProfileDraft[] {
    return profiles.filter(profile => profile.provider === 'openai-compatible').map(profile => ({
        ...profile,
        endpoint: { ...profile.endpoint },
        capabilities: [...profile.capabilities],
        executionCapabilities: profile.executionCapabilities ? { ...profile.executionCapabilities } : profile.executionCapabilities,
        executionCapabilityDraft: profile.executionCapabilityDraft ? { ...profile.executionCapabilityDraft } : undefined,
        parameters: { ...profile.parameters },
        reasoning: profile.reasoning ? { ...profile.reasoning,
            ...(profile.reasoning.efforts ? { efforts: [...profile.reasoning.efforts] } : {}), ...(profile.reasoning.budgets ? { budgets: [...profile.reasoning.budgets] } : {}) } : null,
    }));
}

export function catalogReplacement(saved: RemoteProfile[], draft: RemoteProfile[]) {
    const expectedRevisions = Object.fromEntries(saved.filter(profile => profile.provider === 'openai-compatible')
        .map(profile => [profile.id, profile.revision]));
    const endpoints = groupedProfiles(draft.filter(profile => profile.provider === 'openai-compatible')).map(group => ({
        id: group.endpoint.id,
        name: group.endpoint.name,
        baseURL: group.endpoint.baseURL,
        wireProtocol: group.endpoint.wireProtocol,
        enabled: group.endpoint.enabled,
        models: group.models.map(model => ({
            id: model.id,
            modelName: model.modelName,
            displayName: model.displayName || model.modelName,
            enabled: model.enabled,
            priority: model.priority,
            capabilities: [...model.capabilities],
            executionCapabilities: model.executionCapabilities ? { ...model.executionCapabilities } : null,
            reasoning: model.reasoning ?? null,
            parameters: Object.fromEntries(editableParameters.filter(key => key in model.parameters)
                .map(key => [key, model.parameters[key]])),
        })),
    }));
    return { expectedRevisions, endpoints };
}

export function catalogDraftDirty(saved: RemoteProfile[], draft: RemoteProfile[]): boolean {
    return JSON.stringify(catalogReplacement(saved, saved).endpoints)
        !== JSON.stringify(catalogReplacement(saved, draft).endpoints);
}

/** Validate the complete staged document before offering the atomic save. */
export function catalogDraftValidationError(draft: RemoteProfile[]): ModelValidationIssue | null {
    for (const group of groupedProfiles(draft)) {
        try {
            const snapshot = endpointSnapshot(draft, group.endpoint.id);
            const error = endpointValidationError(snapshot, {
                name: group.endpoint.name,
                baseURL: group.endpoint.baseURL,
                wireProtocol: group.endpoint.wireProtocol,
                enabled: group.endpoint.enabled,
            });
            if (error) return error;
        } catch (error) {
            if (error instanceof ModelValidationError) return error.issue;
            throw error;
        }
        for (const model of group.models) {
            const capabilityError = reasoningExecutionValidationError(model.reasoning, model.executionCapabilities);
            if (capabilityError) return { code: 'modelValidation.endpointReasoning', params: { model: model.displayName }, cause: capabilityError };
            const parameterError = executionParameterValidationError(model);
            if (parameterError) return { code: 'modelValidation.endpointReasoning', params: { model: model.displayName }, cause: parameterError };
        }
    }
    return null;
}

/** Discovery uses persisted endpoint credentials and URL, never an unsaved route. */
export function endpointDiscoveryMatchesSaved(saved: RemoteProfile[], draft: RemoteProfile[], endpointId: string): boolean {
    const savedEndpoint = saved.find(profile => profile.endpoint.id === endpointId)?.endpoint;
    const draftEndpoint = draft.find(profile => profile.endpoint.id === endpointId)?.endpoint;
    return !!savedEndpoint && !!draftEndpoint && savedEndpoint.baseURL === draftEndpoint.baseURL
        && savedEndpoint.wireProtocol === draftEndpoint.wireProtocol;
}

export type DiscoveredModelCatalog = { baseURL: string; wireProtocol: string; models: string[] };

export function discoveredModelsForEndpoint(catalog: DiscoveredModelCatalog | undefined,
    endpoint: RemoteProfile['endpoint']): string[] {
    return catalog?.baseURL === endpoint.baseURL && catalog.wireProtocol === endpoint.wireProtocol
        ? catalog.models : [];
}

export function primaryChatModelId(draft: RemoteProfile[]): string | undefined {
    return groupedProfiles(draft).flatMap(group => group.models)
        .filter(model => model.enabled && model.endpoint.enabled && model.capabilities.includes('chat'))
        .sort((left, right) => right.priority - left.priority || left.endpoint.order - right.endpoint.order
            || left.order - right.order || left.id.localeCompare(right.id))[0]?.id;
}

export function stageEndpoint(draft: RemoteProfile[], id: string, changes: Partial<RemoteProfile['endpoint']>) {
    return draft.map(profile => {
        if (profile.endpoint.id !== id) return profile;
        const routeChanged = changes.baseURL !== undefined && changes.baseURL.trim() !== profile.endpoint.baseURL.trim()
            || changes.wireProtocol !== undefined && changes.wireProtocol !== profile.endpoint.wireProtocol;
        return { ...profile, endpoint: { ...profile.endpoint, ...changes },
            ...(routeChanged ? { executionCapabilities: null,
                executionCapabilityDraft: profile.executionCapabilities ?? (profile as RemoteProfileDraft).executionCapabilityDraft } : {}) };
    });
}

export function stageModel(draft: RemoteProfile[], id: string, changes: Partial<RemoteProfile>) {
    return draft.map(profile => {
        if (profile.id !== id) return profile;
        const routeChanged = changes.modelName !== undefined && changes.modelName.trim() !== profile.modelName.trim();
        return { ...profile, ...changes, ...(routeChanged ? { executionCapabilities: null,
            executionCapabilityDraft: changes.executionCapabilities ?? profile.executionCapabilities
                ?? (profile as RemoteProfileDraft).executionCapabilityDraft } : {}) };
    });
}

export function stageCatalogMove(draft: RemoteProfile[], kind: 'endpoint' | 'model', id: string, direction: -1 | 1) {
    const groups = groupedProfiles(draft);
    if (kind === 'endpoint') {
        const position = groups.findIndex(group => group.endpoint.id === id);
        const destination = position + direction;
        if (position < 0 || destination < 0 || destination >= groups.length) return draft;
        [groups[position], groups[destination]] = [groups[destination], groups[position]];
        const order = new Map(groups.map((group, index) => [group.endpoint.id, index]));
        return draft.map(profile => ({ ...profile, endpoint: { ...profile.endpoint,
            order: order.get(profile.endpoint.id) ?? profile.endpoint.order } }));
    }
    const group = groups.find(group => group.models.some(model => model.id === id));
    if (!group) return draft;
    const position = group.models.findIndex(model => model.id === id);
    const destination = position + direction;
    if (destination < 0 || destination >= group.models.length) return draft;
    [group.models[position], group.models[destination]] = [group.models[destination], group.models[position]];
    const order = new Map(group.models.map((model, index) => [model.id, index]));
    return draft.map(profile => order.has(profile.id) ? { ...profile, order: order.get(profile.id)! } : profile);
}

export function stageCatalogDelete(draft: RemoteProfile[], kind: 'endpoint' | 'model', id: string) {
    const remaining = draft.filter(profile => kind === 'endpoint' ? profile.endpoint.id !== id : profile.id !== id);
    const groups = groupedProfiles(remaining);
    const endpointOrder = new Map(groups.map((group, index) => [group.endpoint.id, index]));
    const modelOrder = new Map(groups.flatMap(group => group.models.map((model, index) => [model.id, index] as const)));
    return remaining.map(profile => ({ ...profile,
        endpoint: { ...profile.endpoint, order: endpointOrder.get(profile.endpoint.id) ?? profile.endpoint.order },
        order: modelOrder.get(profile.id) ?? profile.order,
    }));
}
