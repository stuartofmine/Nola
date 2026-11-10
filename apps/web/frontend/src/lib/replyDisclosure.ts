/** Native disclosure state stays with the reply; animation cannot retain hidden controls. */
export function replyDisclosure(node: HTMLDetailsElement) {
    node.classList.add('reply-disclosure');
    const guarded = new Map<HTMLElement, boolean>();
    function sync() {
        const summary = node.querySelector<HTMLElement>(':scope > summary');
        const active = node.ownerDocument.activeElement;
        // Move focus before applying inert, which otherwise sends it to body.
        if (!node.open && active && node.contains(active) && !summary?.contains(active)) {
            summary?.focus({ preventScroll: true });
        }
        for (const [child, wasInert] of guarded) {
            if (node.open || child.parentElement !== node) {
                child.inert = wasInert;
                guarded.delete(child);
            }
        }
        if (!node.open) for (const child of node.children) {
            if (child instanceof HTMLElement && child !== summary && !guarded.has(child)) {
                guarded.set(child, child.inert);
                child.inert = true;
            }
        }
    }
    // Attribute mutations precede the next input event; toggle alone is too late
    // when content-visibility is deliberately retained for the closing frames.
    const observer = new MutationObserver(sync);
    observer.observe(node, { attributes: true, attributeFilter: ['open'], childList: true });
    sync();
    return { destroy() {
        observer.disconnect();
        for (const [child, wasInert] of guarded) child.inert = wasInert;
        guarded.clear();
        node.classList.remove('reply-disclosure');
    } };
}
