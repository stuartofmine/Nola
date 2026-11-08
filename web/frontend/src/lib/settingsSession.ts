export type SettingsSession = { authenticated: boolean; sessionAuthenticated: boolean; localTrust: boolean; hostId: string; transportIngress: "direct" | "tailcat" };

export function settingsSession(value: unknown): SettingsSession {
    if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('settings.invalidSession');
    const source = value as Record<string, unknown>;
    if (typeof source.authenticated !== 'boolean' || typeof source.sessionAuthenticated !== 'boolean'
        || typeof source.localTrust !== 'boolean' || typeof source.hostId !== 'string' || !source.hostId
        || (source.transportIngress !== 'direct' && source.transportIngress !== 'tailcat')
        || (source.transportIngress === 'tailcat' && source.localTrust)
        || source.authenticated !== (source.sessionAuthenticated || source.localTrust)) throw new Error('settings.invalidSession');
    // Do not retain arbitrary fields, browser tokens or cookie data.
    return { authenticated: source.authenticated, sessionAuthenticated: source.sessionAuthenticated,
        localTrust: source.localTrust, hostId: source.hostId, transportIngress: source.transportIngress };
}

export function logoutConfirmed(value: unknown): boolean {
    return !!value && typeof value === 'object' && !Array.isArray(value) && (value as Record<string, unknown>).authenticated === false;
}
