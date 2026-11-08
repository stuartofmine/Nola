export type CapabilityMode = 'skills' | 'mcp';
export type Capability = Record<string, unknown> & { name?: string; display_name?: string; server_id?: string; revision?: number; enabled?: boolean };
export type CapabilitySettings = { skills: Capability[]; mcp_servers: Capability[]; skill_settings_backup_key?: string };
export function builtin(item: Capability) { return item.built_in === true || item.source === 'built_in' || item.builtin === true || item.isBuiltin === true || item.source === 'builtin'; }
export function capabilityName(item: Capability, fallback = '') { return String(item.display_name || item.name || item.title || item.server_id || fallback); }
export class CapabilityValidationError extends Error {
    readonly key: string;
    constructor(key: string) { super(key); this.key = key; }
}
export function stdioMcp(item: Capability) { return typeof item.command === 'string' || Array.isArray(item.command); }
export function capabilityDraft(item: Capability) {
    return { name: capabilityName(item), description: String(item.description || item.summary || ''),
        primary: stdioMcp(item) ? JSON.stringify({ command: item.command, ...(item.args !== undefined ? { args: item.args } : {}), ...(item.cwd !== undefined ? { cwd: item.cwd } : {}) }, null, 2) : String(item.content || item.instructions || item.markdown || item.endpoint || item.url || ''), enabled: item.enabled !== false };
}
export function capabilityMutation(settings: CapabilitySettings, mode: CapabilityMode, index: number | null, replacement?: Capability) {
    const current = mode === 'skills' ? settings.skills : settings.mcp_servers;
    const updated = current.filter((_, i) => i !== index);
    if (replacement) updated.splice(index ?? updated.length, 0, replacement);
    return mode === 'skills' ? {
        skills: updated,
        skill_baseline: settings.skills.map(({ id, revision }) => ({ id, revision })),
    } : {
        mcp_servers: updated,
        mcp_baseline: settings.mcp_servers.map(({ server_id, revision }) => ({ server_id, revision })),
    };
}
export function updatedCapability(original: Capability, draft: ReturnType<typeof capabilityDraft>, mode: CapabilityMode): Capability {
    if (!draft.name.trim()) throw new CapabilityValidationError('capability.nameRequired');
    if (mode === 'mcp' && stdioMcp(original)) {
        let launch: Record<string, unknown>;
        try {
            launch = JSON.parse(draft.primary);
            const argv = Array.isArray(launch.command) ? launch.command : [launch.command];
            if (!argv.length || !argv.every(value => typeof value === 'string') || !String(argv[0]).trim()
                || (launch.args !== undefined && (!Array.isArray(launch.args) || !launch.args.every(value => typeof value === 'string') || Array.isArray(launch.command)))
                || (launch.cwd !== undefined && typeof launch.cwd !== 'string')
                || Object.keys(launch).some(key => !['command', 'args', 'cwd'].includes(key))) throw new Error();
        } catch { throw new CapabilityValidationError('capability.invalidLaunch'); }
        const result: Capability = { ...original, name: draft.name.trim(), display_name: draft.name.trim(), description: draft.description.trim(), enabled: draft.enabled, ...launch };
        if (launch.args === undefined) delete result.args;
        if (launch.cwd === undefined) delete result.cwd;
        return result;
    }
    if (mode === 'mcp') {
        let url: URL;
        try { url = new URL(draft.primary.trim()); }
        catch { throw new CapabilityValidationError('capability.invalidEndpoint'); }
        if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password) throw new CapabilityValidationError('capability.invalidEndpoint');
    }
    return { ...original, name: draft.name.trim(), description: draft.description.trim(), enabled: draft.enabled,
        ...(mode === 'skills' ? { content: draft.primary } : { display_name: draft.name.trim(), endpoint: draft.primary.trim() }) };
}
export function capabilityCatalog(item: Capability) {
    return ['tools', 'resources', 'resource_templates', 'prompts'].map(key => {
        const value = item[key];
        const entries = value && typeof value === 'object' && !Array.isArray(value) ? Object.entries(value) : [];
        return { key, titleKey: `capability.catalog.${key}`, entries: entries.map(([id, descriptor]) => {
            const data = descriptor && typeof descriptor === 'object' ? descriptor as Record<string, unknown> : {};
            return { id, name: String(data.title || data.name || data.uri || data.uri_template || id), description: String(data.description || '') };
        }) };
    });
}

const categories = [
    'documents', 'data-analysis', 'design-presentations', 'communication',
    'planning-automation', 'knowledge-research', 'workspace', 'custom',
];
export function capabilitySections(entries: { item: Capability; index: number }[], mode: CapabilityMode) {
    const groups = mode === 'mcp' ? ['mcp'] : categories;
    return groups.map(id => ({ id, titleKey: `capability.category.${id}`, entries: entries.filter(({ item }) => {
        if (mode === 'mcp') return true;
        const category = builtin(item) && categories.some(key => key === item.category_id) ? item.category_id : 'custom';
        return category === id;
    }).sort((a, b) => Number(builtin(b.item)) - Number(builtin(a.item)) || capabilityName(a.item).localeCompare(capabilityName(b.item))) }))
        .filter(group => group.entries.length > 0);
}

export function restoredSkill(item: Capability): Capability {
    if (!builtin(item) || typeof item.default_content !== 'string') throw new CapabilityValidationError('capability.noDefaultContent');
    // Restoring instructions is not permission to re-enable a disabled skill.
    return { ...item, content: item.default_content };
}
