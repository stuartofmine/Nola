import { reasoningValidationError, reasoningOptions, type ReasoningConfiguration, type RemoteProfile } from './modelProfiles.ts';
import type { ChatConversation } from './chatConversations.ts';

export type PermissionChoice = 'cautious' | 'allow_all';
export type SessionNumberField = 'temperature' | 'topP' | 'maxOutputTokens' | 'maxContextLength';
export const sessionNumberFields: readonly SessionNumberField[] = ['temperature', 'topP', 'maxOutputTokens', 'maxContextLength'];
export type SessionSeed = { kind: 'clear' } | { kind: 'value'; value: string };
export function validSessionSeedValue(value: string): boolean {
    if (value.length > 20 || /^(?:0|-?[1-9][0-9]*)$/.exec(value)?.[0] !== value) return false;
    const integer = BigInt(value);
    return integer >= -9223372036854775808n && integer <= 9223372036854775807n;
}
function parseSessionSeed(value: unknown): SessionSeed {
    if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('chatSession.invalidSeed');
    const seed = value as Record<string, unknown>;
    if (seed.kind === 'clear' && Object.keys(seed).length === 1) return { kind: 'clear' };
    if (seed.kind === 'value' && Object.keys(seed).length === 2 && typeof seed.value === 'string' && validSessionSeedValue(seed.value)) return { kind: 'value', value: seed.value };
    throw new Error('chatSession.invalidSeed');
}
export type SessionOverrides = Record<string, Record<string, unknown>>;
export interface SessionActivePermissionPolicy { run_id: string; configuration_revision: number; profile: PermissionChoice }
export interface SessionConfiguration { configurationRevision: number; retrievalEnabledOverride: boolean | null; memoryEnabledOverride: boolean | null; permissionProfile: 'cautious' | 'allow_all' | null; modelOverrides: SessionOverrides; permissionState?: 'inactive' | 'pending' | 'active'; activeRunID?: string | null; activePermissionPolicy?: SessionActivePermissionPolicy | null }
export interface SessionDraft { retrieval: 'inherit' | 'on' | 'off'; memory: 'inherit' | 'on' | 'off'; permission: PermissionChoice; reasoningPreference: string; numbers: Record<SessionNumberField, string>; seedMode: 'inherit' | 'clear' | 'value'; seedValue: string }
export interface SessionPermissionModeState {
    choice: PermissionChoice;
    savedChoice: PermissionChoice;
    ready: boolean;
    busy: boolean;
    saving: boolean;
    conflict: boolean;
    canSave: boolean;
    errorKey: string;
    noticeKey: string;
    applicationStatus?: 'draft' | 'saving' | 'saved' | 'pending' | 'active' | 'failed';
}

export interface SessionReasoningQuickState {
    preference: string;
    options: string[];
    ready: boolean;
    busy: boolean;
    errorKey: string;
}

export interface SessionLocalModel {
    id: string;
    name: string;
    installed: boolean;
    running: boolean;
    parameters: Record<string, number | string | boolean | null>;
}

export function parseSessionLocalModels(value: unknown): SessionLocalModel[] {
    const rows = (value as { models?: unknown } | null)?.models;
    if (!Array.isArray(rows)) throw new Error('chatSession.invalidResponse');
    const seen = new Set<string>();
    return rows.map(row => {
        if (!row || typeof row !== 'object' || Array.isArray(row)) throw new Error('chatSession.invalidResponse');
        const model = row as Record<string, unknown>;
        const id = model.id;
        if (typeof id !== 'string' || !id.trim() || seen.has(id)
            || typeof model.installed !== 'boolean' || typeof model.running !== 'boolean'
            || !model.parameters || typeof model.parameters !== 'object' || Array.isArray(model.parameters)) throw new Error('chatSession.invalidResponse');
        seen.add(id);
        return { id, name: typeof model.displayName === 'string' && model.displayName.trim() ? model.displayName
            : typeof model.name === 'string' && model.name.trim() ? model.name : id,
        installed: model.installed, running: model.running,
        parameters: model.parameters as SessionLocalModel['parameters'] };
    });
}

export function localSessionProfile(conversation: ChatConversation, models: SessionLocalModel[]): RemoteProfile | null {
    if (conversation.selectedModelSource !== 'local') return null;
    const model = models.find(item => item.id === conversation.selectedModelId && item.installed);
    if (!model) return null;
    return { id: model.id, displayName: model.name, modelName: model.id, revision: 0,
        enabled: true, selected: false, priority: 0, order: 0,
        endpoint: { id: 'local', name: 'Local', baseURL: '', wireProtocol: 'chat_completions', enabled: true, order: 0 },
        capabilities: ['chat'], reasoning: null,
        parameters: { temperature: model.parameters.temperature ?? null, topP: model.parameters.topP ?? null,
            maxContextLength: model.parameters.contextTokens ?? null } };
}

export function sessionModelSelectionConfirmed(
    conversation: ChatConversation,
    requested: { selectedModelSource: 'auto' | 'cloud' | 'local'; selectedModelId?: string },
): boolean {
    return conversation.selectedModelSource === requested.selectedModelSource
        && (requested.selectedModelSource === 'auto' || conversation.selectedModelId === requested.selectedModelId);
}

export function parseSessionProfiles(value: unknown): RemoteProfile[] {
    const rows = (value as { profiles?: unknown } | null)?.profiles;
    if (!Array.isArray(rows)) throw new Error('chatSession.invalidResponse');
    const seen = new Set<string>();
    for (const row of rows) {
        if (!row || typeof row !== 'object') throw new Error('chatSession.invalidResponse');
        const profile = row as Record<string, unknown>;
        const endpoint = profile.endpoint as Record<string, unknown> | null;
        if (typeof profile.id !== 'string' || !profile.id.trim() || seen.has(profile.id)
            || typeof profile.displayName !== 'string' || typeof profile.modelName !== 'string'
            || typeof profile.enabled !== 'boolean' || !Array.isArray(profile.capabilities)
            || profile.capabilities.some(capability => typeof capability !== 'string')
            || !endpoint || typeof endpoint !== 'object' || Array.isArray(endpoint)
            || typeof endpoint.wireProtocol !== 'string' || typeof endpoint.enabled !== 'boolean'
            || !profile.parameters || typeof profile.parameters !== 'object' || Array.isArray(profile.parameters)) throw new Error('chatSession.invalidResponse');
        const reasoning = profile.reasoning;
        if (reasoning != null && (typeof reasoning !== 'object' || Array.isArray(reasoning)
            || !['automatic', 'effort', 'budget'].includes(String((reasoning as Record<string, unknown>).control))
            || ((reasoning as Record<string, unknown>).efforts !== undefined && (!Array.isArray((reasoning as Record<string, unknown>).efforts) || ((reasoning as Record<string, unknown>).efforts as unknown[]).some(value => typeof value !== 'string')))
            || ((reasoning as Record<string, unknown>).budgets !== undefined && (!Array.isArray((reasoning as Record<string, unknown>).budgets) || ((reasoning as Record<string, unknown>).budgets as unknown[]).some(value => typeof value !== 'number'))))) throw new Error('chatSession.invalidResponse');
        if (reasoning != null && reasoningValidationError(reasoning as ReasoningConfiguration, endpoint.wireProtocol as string, Number((profile.parameters as Record<string, unknown>).maxOutputTokens ?? 16384))) throw new Error('chatSession.invalidResponse');
        seen.add(profile.id);
    }
    return rows as RemoteProfile[];
}

export function parseSessionConfiguration(value: unknown): SessionConfiguration {
    if (!value || typeof value !== 'object') throw new Error('chatSession.invalidResponse');
    const data = value as Record<string, unknown>;
    if (!Number.isSafeInteger(data.configurationRevision) || (data.configurationRevision as number) < 1
        || ![null, true, false].includes(data.memoryEnabledOverride as boolean | null)
        || ![null, true, false].includes(data.retrievalEnabledOverride as boolean | null)
        || ![null, 'cautious', 'allow_all'].includes(data.permissionProfile as string | null)
        || !data.modelOverrides || typeof data.modelOverrides !== 'object' || Array.isArray(data.modelOverrides)) throw new Error('chatSession.invalidResponse');
    const modelOverrides: SessionOverrides = Object.create(null);
    let activePermissionPolicy: SessionActivePermissionPolicy | null = null;
    if (data.permissionState !== undefined && !['inactive', 'pending', 'active'].includes(data.permissionState as string)) throw new Error('chatSession.invalidResponse');
    if (data.activeRunID !== undefined && data.activeRunID !== null && (typeof data.activeRunID !== 'string' || !data.activeRunID.trim())) throw new Error('chatSession.invalidResponse');
    if (data.activePermissionPolicy !== undefined && data.activePermissionPolicy !== null) {
        const policy = data.activePermissionPolicy as Record<string, unknown>;
        if (!policy || typeof policy !== 'object' || Array.isArray(policy) || Object.keys(policy).sort().join(',') !== 'configuration_revision,profile,run_id' ||
            policy.run_id !== data.activeRunID || !Number.isSafeInteger(policy.configuration_revision) || Number(policy.configuration_revision) < 1 ||
            !['cautious', 'allow_all'].includes(String(policy.profile))) throw new Error('chatSession.invalidResponse');
        activePermissionPolicy = { run_id: policy.run_id as string, configuration_revision: policy.configuration_revision as number, profile: policy.profile as PermissionChoice };
    }
    for (const [key, fields] of Object.entries(data.modelOverrides as Record<string, unknown>)) {
        if (!fields || typeof fields !== 'object' || Array.isArray(fields)) throw new Error('chatSession.invalidResponse');
        modelOverrides[key] = { ...fields as Record<string, unknown> };
        if (Object.hasOwn(modelOverrides[key], 'seed')) modelOverrides[key].seed = parseSessionSeed(modelOverrides[key].seed);
    }
    return { configurationRevision: data.configurationRevision as number, retrievalEnabledOverride: data.retrievalEnabledOverride as boolean | null, memoryEnabledOverride: data.memoryEnabledOverride as boolean | null, permissionProfile: data.permissionProfile as SessionConfiguration['permissionProfile'], modelOverrides,
        ...(data.permissionState !== undefined ? { permissionState: data.permissionState as SessionConfiguration['permissionState'] } : {}),
        ...(data.activeRunID !== undefined ? { activeRunID: data.activeRunID as string | null, activePermissionPolicy } : {}) };
}

export function sessionPermissionApplicationStatus(snapshot: SessionConfiguration | null, draft: PermissionChoice | null,
    saving: boolean, failed: boolean, running: boolean, committedPermissionRevision = 0): NonNullable<SessionPermissionModeState['applicationStatus']> {
    if (saving) return 'saving';
    if (failed) return 'failed';
    if (!snapshot || draft !== (snapshot.permissionProfile ?? 'cautious')) return 'draft';
    if (snapshot.permissionState === 'inactive') return 'saved';
    if (snapshot.permissionState === 'active') return 'active';
    return 'pending';
}

export function sessionModelKey(conversation: ChatConversation): string | null {
    if (conversation.selectedModelSource === 'cloud' && conversation.selectedModelId.trim()) return `configured:${conversation.selectedModelId}`;
    if (conversation.selectedModelSource === 'local' && conversation.selectedModelId.trim()) return `local:${conversation.selectedModelId}`;
    return null;
}

export function selectableSessionProfile(conversation: ChatConversation, profiles: RemoteProfile[]): RemoteProfile | null {
    if (conversation.selectedModelSource !== 'cloud') return null;
    const profile = profiles.find(item => item.id === conversation.selectedModelId);
    return profile && profile.enabled && profile.endpoint.enabled !== false && profile.capabilities?.includes('chat') ? profile : null;
}

export function sessionReasoningOptions(profile: RemoteProfile, override: Record<string, unknown> = {}): string[] {
    const protocol = typeof override.wireProtocol === 'string' ? override.wireProtocol : profile.endpoint.wireProtocol;
    const value = override.reasoning && typeof override.reasoning === 'object' && !Array.isArray(override.reasoning)
        ? override.reasoning as ReasoningConfiguration : profile.reasoning;
    if (!value || !['automatic', 'effort', 'budget'].includes(value.control)) return [];
    const options = reasoningOptions(value, protocol, Number(override.maxOutputTokens ?? profile.parameters.maxOutputTokens ?? 16384));
    if (options.length && value.control === 'effort') return [...(value.efforts ?? [])];
    return options;
}

export function sessionDraft(snapshot: SessionConfiguration, key: string | null): SessionDraft {
    const override = key ? snapshot.modelOverrides[key] ?? {} : {};
    const seed = Object.hasOwn(override, 'seed') ? parseSessionSeed(override.seed) : undefined;
    const numbers = Object.fromEntries(sessionNumberFields.map(field => [field,
        typeof override[field] === 'number' ? String(override[field]) : '',
    ])) as Record<SessionNumberField, string>;
    return { retrieval: snapshot.retrievalEnabledOverride === null ? 'inherit' : snapshot.retrievalEnabledOverride ? 'on' : 'off', memory: snapshot.memoryEnabledOverride === null ? 'inherit' : snapshot.memoryEnabledOverride ? 'on' : 'off', permission: snapshot.permissionProfile ?? 'cautious',
        reasoningPreference: typeof override.reasoningPreference === 'string' ? override.reasoningPreference : 'inherit', numbers,
        seedMode: seed?.kind ?? 'inherit', seedValue: seed?.kind === 'value' ? seed.value : '' };
}

export function sessionDraftError(draft: SessionDraft, options: string[]): string | null {
    if (!['inherit', 'on', 'off'].includes(draft.memory)) return 'chatSession.invalidResponse';
    if (!['inherit', 'on', 'off'].includes(draft.retrieval)) return 'chatSession.invalidResponse';
    if (!['cautious', 'allow_all'].includes(draft.permission)) return 'chatSession.invalidPermission';
    if (!['inherit', 'automatic', ...options].includes(draft.reasoningPreference)) return 'chatSession.invalidReasoning';
    if (!['inherit', 'clear', 'value'].includes(draft.seedMode) || draft.seedMode === 'value' && !validSessionSeedValue(draft.seedValue)) return 'chatSession.invalidSeed';
    const limits: Record<SessionNumberField, [number, number, boolean]> = {
        temperature: [0, 2, false], topP: [0, 1, false], maxOutputTokens: [64, 1_000_000, true],
        maxContextLength: [512, 4_000_000, true],
    };
    for (const field of sessionNumberFields) {
        const raw = draft.numbers[field];
        if (raw === '') continue;
        if (!/^[-+]?\d+(?:\.\d+)?$/.test(raw.trim())) return 'chatSession.invalidNumber';
        const number = Number(raw);
        const [min, max, integer] = limits[field];
        if (!Number.isFinite(number) || number < min || number > max || integer && !Number.isSafeInteger(number)) return 'chatSession.invalidNumber';
    }
    return null;
}

export function sessionConfigurationPatch(snapshot: SessionConfiguration, key: string | null, draft: SessionDraft, options: string[]): Record<string, unknown> {
    const problem = sessionDraftError(draft, options);
    if (problem) throw new Error(problem);
    const original = sessionDraft(snapshot, key);
    const changes: Record<string, unknown> = {};
    if (draft.seedMode !== original.seedMode || draft.seedMode === 'value' && draft.seedValue !== original.seedValue) {
        changes.seed = draft.seedMode === 'inherit' ? null : draft.seedMode === 'clear' ? { kind: 'clear' } : { kind: 'value', value: draft.seedValue };
    }
    for (const field of sessionNumberFields) {
        if (draft.numbers[field] !== original.numbers[field]) changes[field] = draft.numbers[field] === '' ? null : Number(draft.numbers[field]);
    }
    if (draft.reasoningPreference !== original.reasoningPreference) {
        changes.reasoningPreference = draft.reasoningPreference === 'inherit' ? null : draft.reasoningPreference;
        if (draft.reasoningPreference !== 'inherit' && key && snapshot.modelOverrides[key]?.reasoningTier != null) changes.reasoningTier = null;
    }
    const patch: Record<string, unknown> = { expectedRevision: snapshot.configurationRevision };
    if (draft.memory !== original.memory) patch.memoryEnabledOverride = draft.memory === 'inherit' ? null : draft.memory === 'on';
    if (draft.retrieval !== original.retrieval) patch.retrievalEnabledOverride = draft.retrieval === 'inherit' ? null : draft.retrieval === 'on';
    if (draft.permission !== original.permission) patch.permissionProfile = draft.permission;
    if (Object.keys(changes).length) {
        if (!key) throw new Error('chatSession.invalidModel');
        patch.modelOverrides = { [key]: changes };
    }
    return patch;
}

export function sessionConfigurationPatchConfirmed(saved: SessionConfiguration, patch: Record<string, unknown>): boolean {
    if (typeof patch.expectedRevision !== 'number' || saved.configurationRevision !== patch.expectedRevision + 1) return false;
    for (const key of ['permissionProfile', 'memoryEnabledOverride', 'retrievalEnabledOverride'] as const) {
        if (Object.hasOwn(patch, key) && saved[key] !== patch[key]) return false;
    }
    if (patch.modelOverrides) {
        for (const [model, fields] of Object.entries(patch.modelOverrides as SessionOverrides)) {
            for (const [key, value] of Object.entries(fields)) {
                const actual = saved.modelOverrides[model]?.[key];
                if (value === null ? actual !== undefined : JSON.stringify(actual) !== JSON.stringify(value)) return false;
            }
        }
    }
    return true;
}

export function sessionPatchHasChanges(patch: Record<string, unknown>): boolean {
    return Object.keys(patch).some(key => key !== 'expectedRevision');
}

/** Permission-only CAS never includes unsaved model parameters. It shares the
 * same canonical revision as the model editor, which must own both views. */
export function sessionPermissionPatch(snapshot: SessionConfiguration, choice: PermissionChoice): Record<string, unknown> | null {
    if (!['cautious', 'allow_all'].includes(choice)) throw new Error('chatSession.invalidPermission');
    const value = choice;
    return value === snapshot.permissionProfile ? null : { expectedRevision: snapshot.configurationRevision, permissionProfile: value };
}

export function permissionPatchConfirmed(snapshot: SessionConfiguration, patch: Record<string, unknown>): boolean {
    return snapshot.configurationRevision > Number(patch.expectedRevision)
        && snapshot.permissionProfile === patch.permissionProfile;
}

export interface SessionModelChoiceState {
    choice: string;
    disabled: boolean;
    errorKey: string;
    options: { value: string; name: string; detail: string; local: boolean }[];
}
