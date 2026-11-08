import type { RemoteProfile } from './modelProfiles';
import { ModelValidationError, type ModelValidationIssue } from './modelValidation.ts';
import { reasoningDraft, reasoningValidationError } from './modelProfiles.ts';

export type EndpointDraft = { name: string; baseURL: string; wireProtocol: string; enabled: boolean };
export type EndpointSnapshot = { id: string; draft: EndpointDraft; profiles: RemoteProfile[]; expectedRevisions: Record<string, number> };

export function validEndpointURL(value: string): boolean {
    try {
        const url = new URL(value.trim());
        return ['https:', 'http:'].includes(url.protocol) && !url.username && !url.password && !value.includes('#');
    } catch { return false; }
}

export function endpointSnapshot(profiles: RemoteProfile[], id: string): EndpointSnapshot {
    const members = profiles.filter(profile => profile.endpoint.id === id);
    if (!id || !members.length) throw new ModelValidationError({ code: 'modelValidation.endpointMissing' });
    if (members.some(profile => !Number.isSafeInteger(profile.revision) || profile.revision < 0)) throw new ModelValidationError({ code: 'modelValidation.endpointRevision' });
    const endpoint = members[0].endpoint;
    // Copy the model contracts as well as revisions. A background refresh must
    // not silently rebase protocol validation for an open endpoint draft.
    return {
        id,
        draft: { name: endpoint.name ?? '', baseURL: endpoint.baseURL ?? '', wireProtocol: endpoint.wireProtocol || 'chat_completions', enabled: endpoint.enabled !== false },
        profiles: members.map(profile => ({ ...profile, endpoint: { ...profile.endpoint }, parameters: { ...profile.parameters }, reasoning: profile.reasoning ? reasoningDraft(profile.reasoning) : null })),
        expectedRevisions: Object.fromEntries(members.map(profile => [profile.id, profile.revision])),
    };
}

export function endpointValidationError(snapshot: EndpointSnapshot, draft: EndpointDraft): ModelValidationIssue | null {
    if (!draft.name.trim()) return { code: 'modelValidation.nameRequired' };
    if (new TextEncoder().encode(draft.name.trim()).length > 512) return { code: 'modelValidation.nameLength', params: { maximum: 512 } };
    if (!validEndpointURL(draft.baseURL)) return { code: 'modelValidation.url' };
    if (!['responses', 'chat_completions', 'anthropic'].includes(draft.wireProtocol)) return { code: 'modelValidation.selectProtocol' };
    for (const model of snapshot.profiles) {
        if (!model.reasoning) continue;
        const error = reasoningValidationError(model.reasoning, draft.wireProtocol, Number(model.parameters.maxOutputTokens ?? 16384));
        if (error) return { code: 'modelValidation.endpointReasoning', params: { model: model.displayName || model.modelName }, cause: error };
    }
    return null;
}

export function endpointRequest(snapshot: EndpointSnapshot, draft: EndpointDraft) {
    const error = endpointValidationError(snapshot, draft);
    if (error) throw new ModelValidationError(error);
    return { expectedRevisions: { ...snapshot.expectedRevisions }, name: draft.name.trim(), baseURL: draft.baseURL.trim(), wireProtocol: draft.wireProtocol, enabled: draft.enabled };
}

export function endpointDirty(snapshot: EndpointSnapshot, draft: EndpointDraft) {
    return (Object.keys(snapshot.draft) as (keyof EndpointDraft)[]).some(key => snapshot.draft[key] !== draft[key]);
}

export function protocolTitle(protocol: string) {
    return ({ responses: 'Responses', chat_completions: 'Chat Completions', anthropic: 'Anthropic Messages' } as Record<string, string>)[protocol] ?? protocol;
}
