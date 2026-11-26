<script lang="ts">
    import { locale, t } from '../lib/i18n';
    import {
        expertAccentPalette,
        expertBubbleColors,
        expertBubblePalette,
        type ExpertBubbleColor,
    } from '../lib/expertBubbleColors';

    let {
        value = 'blue',
        name = '',
        onchange,
    }: {
        value: ExpertBubbleColor;
        name?: string;
        onchange: (next: ExpertBubbleColor) => void;
    } = $props();
</script>

<div class="color-picker" role="radiogroup" aria-label={t($locale, 'expertColor.bubble')}>
    {#each expertBubbleColors as color (color)}
        <button
            type="button"
            role="radio"
            class="swatch-btn"
            class:selected={value === color}
            aria-checked={value === color}
            aria-label={t($locale, `expertColor.${color}`)}
            title={t($locale, `expertColor.${color}`)}
            style:--swatch-accent={expertAccentPalette[color]}
            style:--swatch-light={expertBubblePalette[color][0]}
            onclick={() => onchange(color)}
        >
            <span class="swatch-dot" aria-hidden="true"></span>
            <span class="swatch-label">{t($locale, `expertColor.${color}`)}</span>
        </button>
    {/each}
</div>
{#if name}
    <div
        class="color-preview"
        style:--preview-light={expertBubblePalette[value][0]}
        style:--preview-dark={expertBubblePalette[value][1]}
        style:--preview-accent={expertAccentPalette[value]}
    >
        <span class="preview-avatar" aria-hidden="true">{Array.from(name.trim())[0] ?? 'E'}</span>
        <div class="preview-bubble">
            <strong>{name}</strong>
            <p>{t($locale, 'expertColor.preview')}</p>
        </div>
    </div>
{/if}

<style>
    .color-picker {
        display: flex;
        flex-wrap: wrap;
        gap: 6px;
    }
    .swatch-btn {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        min-height: 30px;
        padding: 4px 9px;
        border: 1px solid var(--border);
        border-radius: 999px;
        background: var(--canvas);
        color: var(--content);
        font-size: 12px;
        cursor: pointer;
    }
    .swatch-btn.selected {
        border-color: var(--swatch-accent);
        box-shadow: 0 0 0 1px var(--swatch-accent);
        font-weight: 600;
    }
    .swatch-dot {
        width: 12px;
        height: 12px;
        border-radius: 999px;
        background: var(--swatch-accent);
        flex: none;
    }
    .color-preview {
        display: flex;
        align-items: flex-start;
        gap: 10px;
        margin-top: 6px;
    }
    .preview-avatar {
        width: 28px;
        height: 28px;
        border-radius: 8px;
        display: grid;
        place-items: center;
        font-size: 12px;
        font-weight: 700;
        color: #fff;
        background: var(--preview-accent);
        flex: none;
    }
    .preview-bubble {
        flex: 1;
        padding: 8px 12px;
        border-radius: 4px 12px 12px 12px;
        border-left: 3px solid var(--preview-accent);
        background: var(--preview-light);
        font-size: 12px;
    }
    .preview-bubble strong {
        display: block;
        margin-bottom: 2px;
    }
    .preview-bubble p {
        margin: 0;
        color: var(--muted);
    }
    @media (prefers-color-scheme: dark) {
        :global(:root:not([data-appearance='light'])) .preview-bubble {
            background: var(--preview-dark);
        }
    }
    :global(:root[data-appearance='dark']) .preview-bubble {
        background: var(--preview-dark);
    }
</style>
