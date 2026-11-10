<script lang="ts">
	import type { Binding } from '../types.js';
	import type { ValidationResult } from '../types.js';
	import ValidationMessages from './ValidationMessages.svelte';

	interface Props {
		bindings: Record<string, Binding>;
        value?: unknown;
		validation: ValidationResult;
		enableDate?: boolean;
		enableTime?: boolean;
		min?: string;
		max?: string;
		label?: string;
		disabled?: boolean;
		weight?: number;
		ariaLabel?: string;
	}

	let {
		bindings,
        value: projectedValue,
		validation,
		enableDate = false,
		enableTime = false,
		min,
		max,
		label = '',
		disabled = false,
		weight,
		ariaLabel
	}: Props = $props();

	const binding = $derived(bindings.value);

	const type = $derived(enableDate && enableTime ? 'datetime-local' : enableTime ? 'time' : 'date');

	/**
	 * The data model holds ISO 8601 (usually UTC); the input elements want local
	 * wall-clock strings. Slicing the ISO string would display UTC digits as if
	 * they were local — type 14:30 in UTC+2, see 12:30 — so instant-bearing
	 * values are converted through Date instead.
	 */
	const pad2 = (n: number) => String(n).padStart(2, '0');

	function localDateTime(iso: string): string {
		const d = new Date(iso);
		if (Number.isNaN(d.getTime())) return '';
		return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}T${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}.${String(d.getMilliseconds()).padStart(3, '0')}`;
	}

	function inputValue(raw: unknown): string {
		if (typeof raw !== 'string' || raw === '') return '';
		return type === 'datetime-local' ? localDateTime(raw) : raw;
	}

	const displayValue = $derived.by(() => {
		return inputValue((binding ? binding.value : projectedValue));
	});

	function write(raw: string) {
		if (raw === '') {
			binding?.set('');
			return;
		}
		if (type === 'time' || type === 'date') {
			binding?.set(raw);
			return;
		}
		const parsed = new Date(raw);
		binding?.set(Number.isNaN(parsed.getTime()) ? raw : parsed.toISOString());
	}
</script>

<label class="a2ui-datetime" style:flex-grow={weight}>
	{#if label}<span class="label">{label}</span>{/if}
	<input
		{type}
		min={min === undefined ? undefined : inputValue(min)}
		max={max === undefined ? undefined : inputValue(max)}
		step="any"
		disabled={disabled || (!enableDate && !enableTime)}
		value={displayValue}
		aria-label={ariaLabel ?? label}
		oninput={(e) => write(e.currentTarget.value)}
	/>
	<ValidationMessages {validation} />
</label>

<style>
	.a2ui-datetime {
		display: flex;
		flex-direction: column;
		gap: 0.25rem;
		margin: var(--a2ui-space-leaf);
		min-width: 0;
	}
	.label {
		font-size: 0.8125rem;
		color: var(--a2ui-color-text-muted);
	}
	input {
		font: inherit;
		color: inherit;
		background: var(--a2ui-color-surface);
		border: 1px solid var(--a2ui-color-border);
		border-radius: var(--a2ui-radius-small);
		padding: 0 0.625rem;
		min-height: var(--a2ui-control-height);
		box-sizing: border-box;
	}
</style>
