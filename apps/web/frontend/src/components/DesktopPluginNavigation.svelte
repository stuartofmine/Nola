<script lang="ts">
    import { onMount } from 'svelte';
    import { locale, t } from '../lib/i18n';
    import {
        desktopPluginCatalog,
        desktopPluginPath,
        type DesktopPluginPage,
    } from '../lib/desktopPluginPages';
    import PrototypeIcon from './PrototypeIcon.svelte';

    let {
        selectedPath = '',
        compact = false,
        onopen,
        onpages,
    }: {
        selectedPath?: string;
        compact?: boolean;
        onopen?: (path: string) => void;
        onpages?: (pages: DesktopPluginPage[], removed: Set<string>) => void;
    } = $props();

    let pages = $state<DesktopPluginPage[]>([]);

    onMount(() => {
        let previous = new Set<string>();
        return desktopPluginCatalog.subscribe(snapshot => {
            pages = snapshot.pages;
            const current = new Set(pages.map(page => desktopPluginPath(page.id)));
            const removed = snapshot.ready
                ? new Set([...previous].filter(path => !current.has(path)))
                : new Set<string>();
            if (snapshot.ready) previous = current;
            onpages?.(pages, removed);
        });
    });

    function handleClick(event: MouseEvent, path: string) {
        if (!onopen) return;
        if (event.button !== 0 || event.ctrlKey || event.metaKey || event.shiftKey || event.altKey) return;
        event.preventDefault();
        onopen(path);
    }
</script>

{#if pages.length}
    <nav class="plugins" class:compact aria-label={t($locale, 'plugin.pages')}>
        {#if !compact}
            <div class="plugins-heading">{t($locale, 'plugin.pages')}</div>
        {/if}
        {#each pages as page (page.id)}
            {@const path = desktopPluginPath(page.id)}
            {@const duplicate = pages.some(other => other.id !== page.id && other.title === page.title)}
            <a
                href={path}
                aria-current={selectedPath === path ? 'page' : undefined}
                title={page.package ? `${page.title} (${page.package})` : page.title}
                onclick={event => handleClick(event, path)}
            >
                <PrototypeIcon name="overview" size={15} />
                {#if !compact}
                    <span>
                        {page.title}
                        {#if duplicate && page.package}
                            <small>{page.package}</small>
                        {/if}
                    </span>
                {/if}
            </a>
        {/each}
    </nav>
{/if}

<style>
    .plugins {
        display: grid;
        gap: 2px;
        flex: 0 1 auto;
        min-height: 0;
        max-height: 30%;
        overflow: auto;
        padding: 6px 2px 2px;
        border-top: 1px solid var(--border);
    }
    .plugins-heading {
        padding: 2px 10px 4px;
        font-size: 11px;
        font-weight: 600;
        color: var(--muted);
        text-transform: uppercase;
        letter-spacing: 0.03em;
    }
    .plugins.compact {
        justify-items: center;
        padding-inline: 0;
    }
    a {
        display: flex;
        align-items: center;
        gap: 8px;
        min-height: 34px;
        padding: 5px 10px;
        border-radius: 8px;
        color: var(--content);
        text-decoration: none;
        font-size: 13px;
    }
    .plugins.compact a {
        width: 36px;
        height: 36px;
        padding: 0;
        justify-content: center;
    }
    a[aria-current],
    a:hover {
        background: var(--composer-pill-hover-bg, var(--canvas));
    }
    a:focus-visible {
        outline: 2px solid var(--primary, var(--accent));
        outline-offset: -2px;
    }
    span {
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    small {
        display: block;
        color: var(--muted);
        font-size: 11px;
    }
</style>
