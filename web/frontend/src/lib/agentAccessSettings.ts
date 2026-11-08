export const permissionProfiles = ['cautious', 'allow_all'] as const;
export type AgentAccessSnapshot = {
    permission_profile: string;
    custom_system_prompt: string;
    enable_local_dev_tools: boolean;
    allowed_dirs: string[];
    bash_prefixes: string[];
};
export type AgentAccessDraft = {
    permissionProfile: string;
    customSystemPrompt: string;
    enableLocalDevTools: boolean;
    allowedDirs: string;
    bashPrefixes: string;
};
export function knownPermissionProfile(value: string): boolean {
    return permissionProfiles.some(profile => profile === value);
}
export function agentAccessSnapshot(value: unknown): AgentAccessSnapshot {
    if (!value || typeof value !== 'object') throw new Error('agentAccess.invalidSnapshot');
    const data = value as Record<string, unknown>;
    const prefixes = data.bash_prefixes;
    const stringArray = (input: unknown): input is string[] => Array.isArray(input) && input.every(item => typeof item === 'string');
    if (typeof data.permission_profile !== 'string' || typeof data.custom_system_prompt !== 'string'
        || typeof data.enable_local_dev_tools !== 'boolean' || !stringArray(data.allowed_dirs) || !stringArray(prefixes)) {
        throw new Error('agentAccess.invalidSnapshot');
    }
    // Unknown permission modes remain visible; they never acquire a permissive default.
    return { permission_profile: data.permission_profile, custom_system_prompt: data.custom_system_prompt,
        enable_local_dev_tools: data.enable_local_dev_tools, allowed_dirs: [...data.allowed_dirs], bash_prefixes: [...prefixes] };
}
export function agentAccessDraft(snapshot: AgentAccessSnapshot): AgentAccessDraft {
    return { permissionProfile: snapshot.permission_profile, customSystemPrompt: snapshot.custom_system_prompt,
        enableLocalDevTools: snapshot.enable_local_dev_tools, allowedDirs: snapshot.allowed_dirs.join('\n'),
        bashPrefixes: snapshot.bash_prefixes.join('\n') };
}
export function agentAccessDirty(snapshot: AgentAccessSnapshot, draft: AgentAccessDraft): boolean {
    return JSON.stringify(agentAccessDraft(snapshot)) !== JSON.stringify(draft);
}
export function agentAccessPatch(snapshot: AgentAccessSnapshot, draft: AgentAccessDraft): Partial<AgentAccessSnapshot> {
    if (!knownPermissionProfile(draft.permissionProfile)) throw new Error('agentAccess.unknownProfile');
    const baseline = agentAccessDraft(snapshot);
    const result: Partial<AgentAccessSnapshot> = {};
    const lines = (value: string) => value.split('\n').map(line => line.trim()).filter(Boolean);
    if (baseline.permissionProfile !== draft.permissionProfile) result.permission_profile = draft.permissionProfile;
    if (baseline.customSystemPrompt !== draft.customSystemPrompt) result.custom_system_prompt = draft.customSystemPrompt;
    if (baseline.enableLocalDevTools !== draft.enableLocalDevTools) result.enable_local_dev_tools = draft.enableLocalDevTools;
    if (baseline.allowedDirs !== draft.allowedDirs) result.allowed_dirs = lines(draft.allowedDirs);
    if (baseline.bashPrefixes !== draft.bashPrefixes) result.bash_prefixes = lines(draft.bashPrefixes);
    return result;
}
export function requiresAllowAllConfirmation(snapshot: AgentAccessSnapshot, draft: AgentAccessDraft): boolean {
    return snapshot.permission_profile !== 'allow_all' && draft.permissionProfile === 'allow_all';
}

/** A well-formed but unchanged response is not proof that the requested write committed. */
export function agentAccessReceipt(value: unknown, submitted: Partial<AgentAccessSnapshot>): AgentAccessSnapshot {
    const snapshot = agentAccessSnapshot(value);
    for (const key of Object.keys(submitted) as (keyof AgentAccessSnapshot)[]) {
        if (JSON.stringify(snapshot[key]) !== JSON.stringify(submitted[key])) throw new Error('agentAccess.saveFailed');
    }
    return snapshot;
}
