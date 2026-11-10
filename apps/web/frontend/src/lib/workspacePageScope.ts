// Renderer-only ownership of mounted editors, DOM associations and native dialogs.
// No product data, execution state, or authorization is stored here.
export type WorkspacePageScope = ReturnType<typeof createWorkspacePageScope>;
export function createWorkspacePageScope(owner: string) {
    const prefix = `workspace-${encodeURIComponent(owner)}-`;
    let active = false, destroyed = false;
    const lifetime = new AbortController();
    let root: HTMLElement | undefined;
    let dialogOrder = 0;
    const dialogs = new Set<{sync: () => void; destroy: () => void; order: () => number}>();
    const id = (value: string) => value.startsWith(prefix) ? value : prefix + value;
    function associate(node: HTMLElement) {
        for (const element of [node, ...node.querySelectorAll<HTMLElement>('[id],[for],[aria-labelledby],[aria-describedby],[aria-controls],[aria-owns],[aria-activedescendant],a[href^="#"]')]) {
            if (element.id && !element.id.startsWith(prefix)) {
                element.dataset.workspaceId = element.id;
                element.id = id(element.id);
            }
            for (const attribute of ['for', 'aria-labelledby', 'aria-describedby', 'aria-controls', 'aria-owns', 'aria-activedescendant']) {
                const value = element.getAttribute(attribute);
                if (value) {
                    const scoped = value.split(/\s+/).map(id).join(' ');
                    if (scoped !== value) element.setAttribute(attribute, scoped);
                }
            }
            const href = element.getAttribute('href');
            if (href?.startsWith('#') && href.length > 1) {
                let fragment = href.slice(1);
                try { fragment = decodeURIComponent(fragment); } catch { /* Invalid URI stays literal. */ }
                if (!fragment.startsWith(prefix)) element.setAttribute('href', '#' + encodeURIComponent(id(fragment)));
            }
        }
    }
    function dom(node: HTMLElement) {
        associate(node);
        const observer = new MutationObserver(() => associate(node));
        observer.observe(node, {subtree: true, childList: true, attributes: true,
            attributeFilter: ['id', 'for', 'aria-labelledby', 'aria-describedby', 'aria-controls', 'aria-owns', 'aria-activedescendant', 'href']});
        return {destroy() { observer.disconnect(); }};
    }
    function mount(node: HTMLElement) {
        root = node;
        const associations = dom(node);
        return {destroy() { associations.destroy(); if (root === node) root = undefined; }};
    }
    function dialog(node: HTMLDialogElement) {
        const associations = dom(node);
        const show = node.showModal.bind(node), close = node.close.bind(node);
        let requested = node.open, suppressedCloses = 0, alive = true;
        let order = node.open ? ++dialogOrder : 0;
        let lastFocus: HTMLElement | undefined;
        function suppressClose(event: Event) {
            if (suppressedCloses) { suppressedCloses--; event.stopImmediatePropagation(); }
            else requested = false;
        }
        node.addEventListener('close', suppressClose, true);
        function sync() {
            if (!alive) return;
            node.inert = !active;
            if (!active && node.open) {
                if (document.activeElement instanceof HTMLElement && node.contains(document.activeElement)) lastFocus = document.activeElement;
                requested = true;
                suppressedCloses++;
                close();
            } else if (active && requested && !node.open && node.isConnected) {
                show();
                if (lastFocus?.isConnected && node.contains(lastFocus)) lastFocus.focus({preventScroll: true});
            }
        }
        node.showModal = () => { if (!alive || destroyed) return; if (!requested) order = ++dialogOrder; requested = true; if (active && !node.open) show(); };
        node.close = value => { requested = false; if (node.open) close(value); };
        const entry = {sync, order: () => order, destroy() {
            if (!alive) return;
            alive = false; requested = false;
            if (node.open) { suppressedCloses++; close(); }
            node.removeEventListener('close', suppressClose, true);
            node.showModal = show; node.close = close;
            associations.destroy(); dialogs.delete(entry);
        }};
        dialogs.add(entry); sync();
        return {destroy: entry.destroy};
    }
    return {
        id, mount, dialog,
        signal: lifetime.signal,
        element: (value: string) => active ? document.getElementById(id(value)) : null,
        query: <T extends Element>(selector: string) => active ? root?.querySelector<T>(selector) ?? null : null,
        setActive(value: boolean) {
            active = value && !destroyed;
            if (root) root.inert = !active;
            // Deactivation closes top-layer dialogs synchronously. Activation
            // waits for the enclosing hidden panel's DOM update.
            if (!active) for (const entry of dialogs) entry.sync();
            else queueMicrotask(() => { if (active && !destroyed) for (const entry of [...dialogs].sort((a, b) => a.order() - b.order())) entry.sync(); });
        },
        destroy() { destroyed = true; active = false; lifetime.abort(); for (const entry of [...dialogs]) entry.destroy(); root = undefined; },
    };
}
