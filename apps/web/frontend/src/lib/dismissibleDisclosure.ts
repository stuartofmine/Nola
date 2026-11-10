/** Native details actions retain normal Tab/Enter semantics and dismiss predictably. */
export function dismissibleDisclosure(node: HTMLDetailsElement) {
    const close = (restore = false) => {
        node.open = false;
        if (restore) node.querySelector('summary')?.focus({preventScroll: true});
    };
    const outside = (event: Event) => {
        if (!node.open || event.composedPath().includes(node)) return;
        // A nested modal owns focus until it closes. Keep its opener available
        // for native focus restoration instead of hiding it behind the modal.
        if ([...document.querySelectorAll('dialog:modal')].some(modal => !modal.contains(node))) return;
        close();
    };
    const keydown = (event: KeyboardEvent) => {
        if (node.open && event.key === 'Escape' && event.composedPath().includes(node)) {
            event.preventDefault();
            event.stopPropagation();
            close(true);
        }
    };
    document.addEventListener('pointerdown', outside);
    document.addEventListener('focusin', outside);
    node.addEventListener('keydown', keydown);
    return {destroy() {
        document.removeEventListener('pointerdown', outside);
        document.removeEventListener('focusin', outside);
        node.removeEventListener('keydown', keydown);
    }};
}
