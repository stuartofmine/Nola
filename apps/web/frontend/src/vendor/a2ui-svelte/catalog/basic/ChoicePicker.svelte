<script lang="ts">
	import { locale, t } from '../../../../lib/i18n';
	import type { Binding } from '../types.js';
	import type { ValidationResult } from '../types.js';
	import ValidationMessages from './ValidationMessages.svelte';

	interface Option {
		label?: string;
		value?: unknown;
	}

	interface Props {
		bindings: Record<string, Binding>;
        value?: unknown;
		validation: ValidationResult;
		options?: Option[];
		label?: string;
		variant?: 'multipleSelection' | 'mutuallyExclusive';
		displayStyle?: 'checkbox' | 'chips';
		filterable?: boolean;
		disabled?: boolean;
		weight?: number;
		ariaLabel?: string;
		a2ui?: { id: string };
	}

	let {
		bindings,
        value: projectedValue,
		validation,
		options = [],
		label = '',
		variant = 'mutuallyExclusive',
		displayStyle = 'checkbox',
		filterable = false,
		disabled = false,
		weight,
		ariaLabel,
		a2ui
	}: Props = $props();

	// Radios need a shared name to form one group for arrow-key navigation.
	const group = $derived(`a2ui-choice-${a2ui?.id ?? 'anon'}`);

	const binding = $derived(bindings.value);

	// The wire format stores an array even in single-select mode.
	const selected = $derived.by(() => {
		const raw = (binding ? binding.value : projectedValue);
		if (Array.isArray(raw)) return raw;
		return raw === null || raw === undefined ? [] : [raw];
	});

	let filter = $state('');
	const visible = $derived(
		filterable && filter.trim() !== ''
			? options.filter((o) =>
					String(o.label ?? o.value ?? '')
						.toLowerCase()
						.includes(filter.toLowerCase())
				)
			: options
	);

	function toggle(value: unknown) {
		if (variant === 'mutuallyExclusive') {
			binding?.set([value]);
			return;
		}
		const next = selected.includes(value)
			? selected.filter((v) => v !== value)
			: [...selected, value];
		binding?.set(next);
	}
</script>

<fieldset class="a2ui-choice" style:flex-grow={weight} {disabled} aria-label={ariaLabel ?? label}>
	{#if label}<legend>{label}</legend>{/if}

	{#if filterable}
		<input
			class="filter"
			type="search"
			placeholder={`${t($locale, 'automation.format.search')}…`}
			aria-label={ariaLabel || label || t($locale, 'automation.format.search')}
			bind:value={filter}
		/>
	{/if}

	<div class="options" class:chips={displayStyle === 'chips'}>
		{#each visible as option, i (option.value ?? i)}
			{#if displayStyle === 'chips'}
				<button
					type="button"
					class="chip"
					class:selected={selected.includes(option.value)}
					aria-pressed={selected.includes(option.value)}
					onclick={() => toggle(option.value)}
				>
					{option.label ?? String(option.value ?? '')}
				</button>
			{:else}
				<label class="option">
					<input
						type={variant === 'mutuallyExclusive' ? 'radio' : 'checkbox'}
						name={variant === 'mutuallyExclusive' ? group : undefined}
						checked={selected.includes(option.value)}
						onchange={() => toggle(option.value)}
					/>
					<span>{option.label ?? String(option.value ?? '')}</span>
				</label>
			{/if}
		{/each}
	</div>

	<ValidationMessages {validation} />
</fieldset>

<style>
	.a2ui-choice {
		border: none;
		padding: 0;
		margin: var(--a2ui-space-leaf);
		min-width: 0;
	}
	legend {
		padding: 0;
		font-size: 0.8125rem;
		color: var(--a2ui-color-text-muted);
	}
	.filter {
		font: inherit;
		color: inherit;
		background: var(--a2ui-color-surface);
		border: 1px solid var(--a2ui-color-border);
		border-radius: var(--a2ui-radius-small);
		padding: 0.25rem 0.5rem;
		margin: 0.25rem 0;
		width: 100%;
		box-sizing: border-box;
	}
	.options {
		display: flex;
		flex-direction: column;
		gap: 0.25rem;
		margin-top: 0.25rem;
	}
	.options.chips {
		flex-direction: row;
		flex-wrap: wrap;
		gap: 0.375rem;
	}
	.option {
		display: inline-flex;
		align-items: center;
		gap: 0.5rem;
		cursor: pointer;
	}
	.option input {
		accent-color: var(--a2ui-color-primary);
	}
	.chip {
		font: inherit;
		color: var(--a2ui-color-text);
		background: var(--a2ui-color-surface);
		border: 1px solid var(--a2ui-color-border);
		border-radius: 999px;
		padding: 0.25rem 0.75rem;
		cursor: pointer;
	}
	.chip:hover:not(:disabled) {
		border-color: var(--outline, var(--a2ui-color-primary));
		background: var(--control-hover-bg, var(--a2ui-color-surface-raised));
	}
	.chip.selected,
	.chip.selected:hover:not(:disabled) {
		background: var(--a2ui-color-accent, var(--a2ui-color-primary));
		border-color: var(--a2ui-color-accent, var(--a2ui-color-primary));
		color: var(--a2ui-color-on-primary);
		font-weight: 550;
	}
</style>
