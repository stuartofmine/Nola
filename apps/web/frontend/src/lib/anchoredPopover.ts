/** Keep a details-owned action popup in the viewport and above content paint layers. */
export function anchoredPopover(node: HTMLElement) {
    const owner = node.closest('details');
    const anchor = owner?.querySelector('summary');
    if (!owner || !anchor) return {};
    function place() {
        if (!node.matches(':popover-open')) return;
        const rect = anchor!.getBoundingClientRect();
        if (!anchor!.getClientRects().length || rect.bottom <= 0 || rect.top >= window.innerHeight || rect.right <= 0 || rect.left >= window.innerWidth) {
            owner!.open = false; node.hidePopover(); return;
        }
        const width = Math.min(320, window.innerWidth - 24);
        const below = Math.max(0, window.innerHeight - rect.bottom - 16);
        const above = Math.max(0, rect.top - 16);
        const upward = below < Math.min(node.scrollHeight, 180) && above > below;
        const height = Math.min(260, upward ? above : below);
        node.style.width = `${width}px`;
        node.style.maxHeight = `${Math.max(44, height)}px`;
        node.style.left = `${Math.max(12, Math.min(rect.right - width, window.innerWidth - width - 12))}px`;
        node.style.top = `${upward ? Math.max(12, rect.top - Math.min(node.scrollHeight, Math.max(44, height)) - 4) : Math.min(rect.bottom + 4, window.innerHeight - 56)}px`;
    }
    function sync() {
        if (owner!.open) { if (!node.matches(':popover-open')) node.showPopover(); place(); }
        else if (node.matches(':popover-open')) node.hidePopover();
    }
    owner.addEventListener('toggle', sync);
    window.addEventListener('resize', place);
    document.addEventListener('scroll', place, true);
    sync();
    return { destroy() {
        owner.removeEventListener('toggle', sync);
        window.removeEventListener('resize', place);
        document.removeEventListener('scroll', place, true);
        if (node.matches(':popover-open')) node.hidePopover();
    } };
}
