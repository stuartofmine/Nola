/** Durable, Host-scoped browser authority. A tab must never invent an ephemeral owner. */
export const browserIdentityKey = (hostId: string) => `nola.web.chat.client.${JSON.stringify(hostId)}`;
const valid = (value: string) => /^[A-Za-z0-9-]{8,100}$/.test(value);
export function readBrowserClientId(hostId: string, storage: Pick<Storage, 'getItem'> = localStorage): string | null {
    if (!hostId.trim()) throw new Error('browser_recovery_storage_unavailable');
    const value = storage.getItem(browserIdentityKey(hostId));
    if (value !== null && !valid(value)) throw new Error('browser_recovery_storage_unavailable');
    return value;
}
export async function ensureBrowserClientId(hostId: string): Promise<string> {
    if (typeof navigator === 'undefined' || !navigator.locks) throw new Error('browser_secure_connection_required');
    try {
        return await navigator.locks.request(browserIdentityKey(hostId), () => {
            const saved = readBrowserClientId(hostId);
            const value = saved ?? `web-${crypto.randomUUID()}`;
            // Rewriting the same value verifies that intents can also be retained.
            localStorage.setItem(browserIdentityKey(hostId), value);
            if (readBrowserClientId(hostId) !== value) throw new Error('storage verification failed');
            return value;
        });
    } catch { throw new Error('browser_recovery_storage_unavailable'); }
}
