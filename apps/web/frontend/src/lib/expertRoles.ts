import type {RemoteProfile} from './modelProfiles.ts';
export type ExpertLocalizedText = Record<string, string>;
export type ExpertRoleTemplate = {id: string; role_id: string; name: ExpertLocalizedText; description: ExpertLocalizedText;
    prompt: string; tool_pack: {id: string; tools: string[]}; skills: {id: string; name: ExpertLocalizedText; instructions: string}[]};
export class ExpertRoleCatalogUnavailable extends Error {}
const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);
const text = (value: unknown): value is string => typeof value === 'string' && !!value.trim();
const localized = (value: unknown): value is ExpertLocalizedText => object(value) && Object.keys(value).length > 0 && Object.values(value).every(text);
export function parseExpertRoles(value: unknown): ExpertRoleTemplate[] {
    if (!object(value) || value.version !== 1 || !object(value.features) || value.features.reasoning_effort !== true
        || value.features.explicit_generation_reasoning !== true || value.features.rejoin !== true || !Array.isArray(value.templates)) throw new ExpertRoleCatalogUnavailable();
    const roles = value.templates;
    for (const role of roles) {
        if (!object(role) || !text(role.id) || !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(role.id)
            || !text(role.role_id) || !localized(role.name) || !localized(role.description) || !text(role.prompt)
            || !object(role.tool_pack) || !text(role.tool_pack.id) || !Array.isArray(role.tool_pack.tools) || !role.tool_pack.tools.every(text)
            || !Array.isArray(role.skills) || !role.skills.every(skill => object(skill) && text(skill.id) && localized(skill.name) && text(skill.instructions))) {
            throw new ExpertRoleCatalogUnavailable();
        }
    }
    if (new Set(roles.map(role => role.id)).size !== roles.length) throw new ExpertRoleCatalogUnavailable();
    return structuredClone(roles) as ExpertRoleTemplate[];
}
export function expertLocalized(value: ExpertLocalizedText, language: string): string {
    return value[language] ?? value[language.split('-')[0]] ?? value.en ?? Object.values(value)[0] ?? '';
}
/** Efforts are protocol values declared by the effective model, never guessed tiers. */
export function expertEfforts(profiles: RemoteProfile[], selection: string, defaultProfileId?: string | null): string[] {
    const profile = selection ? profiles.find(profile => `configured:${profile.id}` === selection) : defaultProfileId ? profiles.find(profile => profile.id === defaultProfileId) : undefined;
    if (!profile || !profile.enabled || !profile.endpoint.enabled || profile.executionCapabilities?.reasoning !== true || profile.reasoning?.control !== 'effort') return [];
    return ['none','minimal','low','medium','high','xhigh','max'].filter(value => profile.reasoning?.efforts?.includes(value));
}

export function expertDefaultProfileId(value: unknown): string | null {
    return object(value) && value.kind === 'resolved' && text(value.profile_id) ? value.profile_id : null;
}

/** Additive capability: old Hosts may still edit/read roles, but cannot consume Main selections. */
export function supportsMainExpertDelegation(value:unknown):boolean {
    return object(value)&&value.version===1&&object(value.features)&&value.features.main_agent_delegation===true;
}
