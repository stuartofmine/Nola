<script lang="ts">
    let { label, controls, value, min, max, defaultValue, reverse = false, onchange }: {
        label: string; controls?: string; value: number; min: number; max: number; defaultValue: number; reverse?: boolean; onchange: (value: number) => void;
    } = $props();
    let drag: {id: number; x: number; width: number} | null = null;
    const change = (next: number) => onchange(Math.round(Math.max(min, Math.min(max, next))));
    function start(event: PointerEvent) {
        if (event.button !== 0) return;
        event.preventDefault();
        const handle = event.currentTarget as HTMLElement;
        handle.focus(); handle.setPointerCapture(event.pointerId);
        drag = {id: event.pointerId, x: event.clientX, width: value};
    }
    function move(event: PointerEvent) {
        if (drag?.id === event.pointerId) change(drag.width + (event.clientX - drag.x) * (reverse ? -1 : 1));
    }
    function end(event: PointerEvent) {
        if (drag?.id !== event.pointerId) return;
        drag = null;
        const handle = event.currentTarget as HTMLElement;
        if (handle.hasPointerCapture(event.pointerId)) handle.releasePointerCapture(event.pointerId);
    }
    function key(event: KeyboardEvent) {
        if (event.altKey || event.ctrlKey || event.metaKey) return;
        const step = event.shiftKey ? 40 : 10;
        if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
            event.preventDefault(); change(value + (event.key === 'ArrowRight' ? step : -step) * (reverse ? -1 : 1));
        } else if (['Home', 'End', 'Enter'].includes(event.key)) {
            event.preventDefault(); change(event.key === 'Home' ? min : event.key === 'End' ? max : defaultValue);
        }
    }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions (This is a focusable window splitter with value semantics and keyboard resizing, not a static separator.) -->
<div class="resize-handle" class:reverse role="separator" tabindex="0" aria-label={label} aria-controls={controls} aria-orientation="vertical"
    aria-valuemin={min} aria-valuemax={max} aria-valuenow={Math.round(value)}
    onpointerdown={start} onpointermove={move} onpointerup={end} onpointercancel={end} onlostpointercapture={() => drag = null}
    onkeydown={key} ondblclick={() => change(defaultValue)}></div>

<style>
    .resize-handle { position: absolute; z-index: 2; top: 0; bottom: 0; right: -7px; width: 14px; cursor: col-resize; touch-action: none; }
    .resize-handle.reverse { right: auto; left: -7px; }
    .resize-handle::after { content: ''; position: absolute; top: 12px; bottom: 12px; left: 6px; width: 2px; border-radius: 2px; background: transparent; }
    .resize-handle:hover::after, .resize-handle:focus-visible::after { background: var(--accent); }
    .resize-handle:focus-visible { outline: 1px solid var(--accent); outline-offset: -2px; border-radius: 4px; }
</style>
