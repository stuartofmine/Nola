// On touch browsers a tap outside an editor does not always move DOM focus.
const controls = 'a[href], button, input, textarea, select, label, summary, [contenteditable]:not([contenteditable="false"]), [role="button"], [role="tab"], [role="switch"], [role="option"], [role="menuitem"]';

function textEditor(element: Element | null): HTMLElement | null {
    if (element instanceof HTMLTextAreaElement) return element;
    if (element instanceof HTMLInputElement && ['text', 'search', 'password', 'email', 'url', 'tel', 'number'].includes(element.type)) return element;
    return element instanceof HTMLElement && element.isContentEditable ? element : null;
}

export function dismissKeyboardOnBackgroundTap() {
    let tap: {id: number; x: number; y: number; time: number; editor: HTMLElement} | undefined;
    const listeners = new AbortController();
    const options = {capture: true, passive: true, signal: listeners.signal};
    document.addEventListener('pointerdown', event => {
        tap = undefined;
        if (event.pointerType !== 'touch' || !event.isPrimary) return;
        const editor = textEditor(document.activeElement);
        if (!editor || event.composedPath().some(node => node instanceof Element && node.matches(controls))) return;
        tap = {id: event.pointerId, x: event.clientX, y: event.clientY, time: event.timeStamp, editor};
    }, options);
    document.addEventListener('pointermove', event => {
        if (tap?.id === event.pointerId && Math.hypot(event.clientX - tap.x, event.clientY - tap.y) > 10) tap = undefined;
    }, options);
    document.addEventListener('pointercancel', () => { tap = undefined; }, options);
    document.addEventListener('pointerup', event => {
        const completed = tap;
        tap = undefined;
        if (!completed || completed.id !== event.pointerId || completed.editor !== document.activeElement
            || Math.hypot(event.clientX - completed.x, event.clientY - completed.y) > 10
            || event.timeStamp - completed.time > 450 || document.getSelection()?.isCollapsed === false) return;
        // Touch pointers capture their initial element. Hit-test the release position too.
        const target = document.elementFromPoint(event.clientX, event.clientY);
        if (!target || target.closest(controls)) return;
        completed.editor.blur();
    }, options);
    return () => listeners.abort();
}
