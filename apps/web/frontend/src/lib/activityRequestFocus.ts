import { tick } from 'svelte';
import { activityFocusOwner } from './activity';

type RequestOwner = {conversationId: string; runId: string | null | undefined; active: boolean};
/** Focus a rendered, current request card; never choose or submit an action. */
export function activityRequestFocus(node: HTMLElement, owner: RequestOwner) {
    let disposed = false;
    const focus = () => {
        const url = new URL(window.location.href), target = activityFocusOwner(url);
        if (disposed || !owner.active || !target || target.conversationId !== owner.conversationId || target.runId !== owner.runId
            || !node.isConnected || node.closest('[hidden], [inert]') || document.querySelector('dialog:modal') || !node.getClientRects().length) return;
        node.scrollIntoView({block: 'center', behavior: 'instant'});
        node.focus({preventScroll: true});
        if (document.activeElement !== node) return;
        url.searchParams.delete('focus'); url.searchParams.delete('request_conversation');
        window.history.replaceState(window.history.state, '', url);
    };
    const schedule = () => { void tick().then(focus); };
    document.addEventListener('close', schedule, true);
    schedule();
    return {update(next: RequestOwner) { owner = next; schedule(); },
        destroy() { disposed = true; document.removeEventListener('close', schedule, true); }};
}
