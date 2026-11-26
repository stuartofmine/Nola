export type SettingsSession = { authenticated: boolean; sessionAuthenticated: boolean; localTrust: boolean; bearerAuthenticated: boolean; hostId: string; transportIngress: "direct" | "lan" | "tailcat" };

export function settingsSession(value: unknown): SettingsSession {
    if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('settings.invalidSession');
    const source = value as Record<string, unknown>;
    if (source.bearerAuthenticated !== undefined && typeof source.bearerAuthenticated !== 'boolean') {
        throw new Error('settings.invalidSession');
    }
    const bearerAuthenticated = source.bearerAuthenticated === true;
    if (typeof source.authenticated !== 'boolean' || typeof source.sessionAuthenticated !== 'boolean'
        || typeof source.localTrust !== 'boolean' || typeof source.hostId !== 'string' || !source.hostId
        || (source.transportIngress !== 'direct' && source.transportIngress !== 'lan' && source.transportIngress !== 'tailcat')
        || (source.transportIngress === 'tailcat' && source.localTrust)
        || source.authenticated !== (source.sessionAuthenticated || source.localTrust || bearerAuthenticated)) throw new Error('settings.invalidSession');
    // Do not retain arbitrary fields, browser tokens or cookie data.
    return { authenticated: source.authenticated, sessionAuthenticated: source.sessionAuthenticated,
        localTrust: source.localTrust, bearerAuthenticated, hostId: source.hostId, transportIngress: source.transportIngress };
}

export function logoutConfirmed(value: unknown): boolean {
    return !!value && typeof value === 'object' && !Array.isArray(value) && (value as Record<string, unknown>).authenticated === false;
}
