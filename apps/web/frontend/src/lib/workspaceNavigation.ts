import { managementContentPath } from './conversationContentTabs.ts';
import { pageLoader } from './pageRoutes.ts';

export const workspaceContentNavigationEvent = 'nola:workspace-content-navigation';
export const workspaceNavigationEvent = 'nola:workspace-navigation';
export function workspaceDestination(href: string, current: string): URL | null {
    const base = new URL(current);
    const target = new URL(href, base);
    if (target.origin !== base.origin || target.username || target.password || target.pathname === '/login'
        || base.pathname === '/login' || !pageLoader(target.pathname)) return null;
    // Preserve normal fragment navigation, including Markdown links within a page.
    if (target.pathname === base.pathname && target.search === base.search && target.hash) return null;
    return target;
}
export function workspaceClick(event: MouseEvent, anchor: HTMLAnchorElement): boolean {
    return !event.defaultPrevented && event.button === 0 && !event.metaKey && !event.ctrlKey && !event.shiftKey && !event.altKey
        && !anchor.hasAttribute('download') && (!anchor.target || anchor.target === '_self')
        && !anchor.relList.contains('external');
}
export function navigateWorkspace(href: string): void {
    const event = new CustomEvent(workspaceNavigationEvent, { detail: href, cancelable: true });
    if (window.dispatchEvent(event)) window.location.assign(href);
}

/** Resolve the current layout before mounting a page, avoiding a chat flash on phones. */
export function initialWorkspaceLocation(href: string, width: number): URL {
    const location = new URL(href);
    if (!Number.isFinite(width) || width < 0) return location;
    if (width <= 700 && location.pathname === '/chat') {
        const content = location.searchParams.get('content');
        const management = content ? managementContentPath(content, location.origin) : null;
        if (management) return new URL(management.path, location.origin);
        if (!location.search) location.pathname = '/conversations';
    } else if (width > 700 && managementContentPath(location.href, location.origin)) {
        const content = location.pathname + location.search + location.hash;
        location.pathname = '/chat'; location.search = ''; location.hash = '';
        location.searchParams.set('content', content);
    }
    return location;
}
