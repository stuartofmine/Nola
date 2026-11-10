import { isVerifiedTerminal } from './assistantTerminal.ts';
import type { ChatTurnOwner } from './chatTurn.ts';
import { api } from './api.ts';
import { navigateWorkspace } from './workspaceNavigation.ts';

export const notificationPreferenceKey = 'nola.browserNotifications.enabled';
const seenPreferenceKey = 'nola.browserNotifications.seen';
type Environment = {
    supported(): boolean; permission(): NotificationPermission; hidden(): boolean;
    read(): string | null; write(value: string): void;
    readSeen(): string | null; writeSeen(value: string): void;
    request(): Promise<NotificationPermission>;
    show(title: string, body: string, activate?: () => void): void;
    activate(owner: ChatTurnOwner): Promise<void>;
};

export function notificationConversationHref(owner: ChatTurnOwner, session: unknown): string | null {
    if (!session || typeof session !== 'object' || !('authenticated' in session) || session.authenticated !== true
        || !('hostId' in session) || session.hostId !== owner.hostId || !owner.hostId || !owner.conversationId) return null;
    const query = new URLSearchParams({ conversation: owner.conversationId, sourceHost: owner.hostId });
    return `/chat?${query}`;
}
export function createBrowserNotifications(environment: Environment) {
    const seen = new Set<string>();
    let restored = false;
    const restoreSeen = () => {
        if (restored) return;
        restored = true;
        try {
            const raw = environment.readSeen();
            if (!raw || raw.length > 1_000_000) return;
            const values: unknown = JSON.parse(raw);
            if (!Array.isArray(values) || values.length > 200) return;
            for (const value of values) {
                if (typeof value !== 'string' || value.length > 4096) continue;
                const scope: unknown = JSON.parse(value);
                if (Array.isArray(scope) && scope.length === 4 && scope.every(id => typeof id === 'string' && id.length > 0 && id.length <= 512)) seen.add(value);
            }
        } catch { /* Restricted/corrupt tab storage retains in-memory deduplication. */ }
    };
    const snapshot = () => {
        const supported = environment.supported();
        let enabled = false;
        try { enabled = environment.read() === 'true'; } catch { /* Optional preference is unavailable. */ }
        return { supported, enabled, permission: supported ? environment.permission() : 'unsupported' as const };
    };
    return {
        snapshot,
        async setEnabled(enabled: boolean) {
            if (enabled) {
                if (!environment.supported()) throw new Error('Notifications unavailable');
                const permission = environment.permission() === 'default' ? await environment.request() : environment.permission();
                if (permission !== 'granted') return false;
            }
            environment.write(String(enabled));
            return enabled;
        },
        test(title: string, body: string) {
            const state = snapshot();
            if (!state.supported || state.permission !== 'granted') throw new Error('Notifications unavailable');
            environment.show(title, body);
        },
        completed(evidence: unknown, owner: ChatTurnOwner, title: string, body: string): boolean {
            if (!isVerifiedTerminal(evidence) || evidence.owner.hostId !== owner.hostId
                || evidence.owner.conversationId !== owner.conversationId || evidence.owner.clientId !== owner.clientId
                || evidence.owner.generation !== owner.generation) return false;
            const key = JSON.stringify([owner.hostId, owner.clientId, owner.conversationId, evidence.seal.identity.run_id]);
            restoreSeen();
            if (seen.has(key)) return false;
            seen.add(key);
            if (seen.size > 200) seen.delete(seen.values().next().value!);
            try { environment.writeSeen(JSON.stringify([...seen])); } catch { /* Optional tab persistence. */ }
            const state = snapshot();
            if (!state.enabled || state.permission !== 'granted' || !environment.hidden()) return false;
            // Capture the verified destination, never a later mutable selection.
            const destination = { ...owner };
            let activated = false;
            try {
                environment.show(title, body, () => {
                    if (activated) return;
                    activated = true;
                    void environment.activate(destination).catch(() => { /* Focus remains available; never navigate on an unverified session. */ });
                });
                return true;
            } catch { return false; }
        },
    };
}

export const browserNotifications = createBrowserNotifications({
    supported: () => typeof window !== 'undefined' && window.isSecureContext && 'Notification' in window,
    permission: () => Notification.permission,
    hidden: () => document.hidden,
    read: () => localStorage.getItem(notificationPreferenceKey),
    write: value => localStorage.setItem(notificationPreferenceKey, value),
    // Opaque completion identities only; no titles, prompts, results or tokens.
    // Tab-scoped so reload can deduplicate without a durable transcript copy.
    readSeen: () => sessionStorage.getItem(seenPreferenceKey),
    writeSeen: value => sessionStorage.setItem(seenPreferenceKey, value),
    request: () => Notification.requestPermission(),
    show: (title, body, activate) => {
        const notification = new Notification(title, { body });
        notification.onclick = event => {
            event.preventDefault();
            window.focus();
            notification.close();
            activate?.();
        };
    },
    activate: async owner => {
        const href = notificationConversationHref(owner, await api('/api/web/session'));
        if (href) navigateWorkspace(href);
    },
});
