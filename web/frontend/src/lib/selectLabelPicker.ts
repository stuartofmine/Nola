/** Keep native selects and keyboard behavior; make their associated labels open
 * the picker when a browser supports showPicker in a trusted user gesture. */
export function installSelectLabelPickers(root: Document): () => void {
    const clicked = (event: MouseEvent) => {
        if (!event.isTrusted || event.defaultPrevented || event.button !== 0 || !(event.target instanceof Element)) return;
        const label = event.target.closest('label');
        const select = label?.control;
        if (!(select instanceof HTMLSelectElement) || select.matches(':disabled')
            || event.target.closest('a, button, input, select, textarea, summary, [contenteditable]')) return;
        // Native label activation still focuses on browsers without showPicker.
        if (typeof select.showPicker !== 'function') return;
        try {
            select.focus({ preventScroll: true });
            select.showPicker();
            event.preventDefault();
        } catch {
            // Unsupported presentation (e.g. a restricted frame) retains the
            // browser's normal label activation; never manufacture a selection.
        }
    };
    root.addEventListener('click', clicked);
    return () => root.removeEventListener('click', clicked);
}
