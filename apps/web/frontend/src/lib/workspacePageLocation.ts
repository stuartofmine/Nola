import type { WorkspacePageScope } from './workspacePageScope';
import { getContext } from 'svelte';

export const workspacePageLocationContext = Symbol('workspace-page-location');
export type WorkspacePageLocation = { key?: string; scope?: WorkspacePageScope; read: () => URL; active: () => boolean; replace: (url: URL) => void };
/** Embedded management pages must not rewrite their enclosing conversation URL. */
export function workspacePageLocation(): WorkspacePageLocation {
    return getContext<WorkspacePageLocation | undefined>(workspacePageLocationContext) ?? {
        read: () => new URL(typeof window === 'undefined' ? 'http://nola.invalid/' : window.location.href),
        active: () => true,
        replace: url => window.history.replaceState(window.history.state, '', url),
    };
}

const tabCloseOwner = Symbol('workspace-tab-close-owner');
type ScopedUnloadEvent = Event & {[tabCloseOwner]?: string};
/** A tab close asks only that editor; browser exit still asks every editor. */
export function workspaceUnloadScope(): (event: Event) => boolean {
    const owner = getContext<WorkspacePageLocation | undefined>(workspacePageLocationContext)?.key;
    return event => {
        const requested = (event as ScopedUnloadEvent)[tabCloseOwner];
        return requested === undefined || requested === owner;
    };
}
export function workspaceTabCloseEvent(key: string): Event {
    const event: ScopedUnloadEvent = new Event('beforeunload', {cancelable: true});
    event[tabCloseOwner] = key;
    return event;
}

/** Native modal ownership also follows its Svelte owner after portalling. */
export function workspaceDialog() {
    const scope = getContext<WorkspacePageLocation | undefined>(workspacePageLocationContext)?.scope;
    return (node: HTMLDialogElement) => scope?.dialog(node) ?? {destroy() {}};
}
export function workspaceElements() {
    const page = workspacePageLocation();
    return {
        element: (id: string) => page.scope ? page.scope.element(id) : page.active() ? document.getElementById(id) : null,
        query: <T extends Element>(selector: string) => page.scope ? page.scope.query<T>(selector) : page.active() ? document.querySelector<T>(selector) : null,
    };
}

/** Requests remain with the mounted editor and abort only when its owner is forgotten. */
export function workspacePageApi(base: typeof import('./api').api) {
    const scope = workspacePageLocation().scope;
    return <T>(path: string, options: RequestInit = {}): Promise<T> => base<T>(path, scope ? {
        ...options, signal: options.signal ? AbortSignal.any([scope.signal, options.signal]) : scope.signal,
    } : options);
}
