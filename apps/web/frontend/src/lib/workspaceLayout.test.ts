import test from 'node:test';
import assert from 'node:assert/strict';
import { get } from 'svelte/store';
import { loadWorkspaceWidths, workspaceWidths, panelWidth, visiblePanelWidths, layoutDefaults } from './workspaceLayout.ts';

test('layout preferences accept finite numeric widths only and clamp supported ranges', () => {
    for (const value of [null, undefined, '360', Infinity, NaN, {}]) assert.equal(panelWidth('sidebar', value), 224);
    assert.equal(panelWidth('sidebar', 10), 220);
    assert.equal(panelWidth('sidebar', 9999), 360);
    assert.equal(panelWidth('inspector', 9999), 480);
    loadWorkspaceWidths({getItem: () => '{invalid'});
    assert.deepEqual(get(workspaceWidths), layoutDefaults);
    loadWorkspaceWidths({getItem: () => JSON.stringify({sidebar: 290, inspector: 410, secret: 'ignored'})});
    assert.deepEqual(get(workspaceWidths), {sidebar: 290, inspector: 410});
    loadWorkspaceWidths({getItem: () => {throw new Error('denied');}});
    assert.deepEqual(get(workspaceWidths), layoutDefaults);
});
test('narrow windows constrain rendering without changing saved widths', () => {
    const saved = {sidebar: 360, inspector: 480};
    for (const viewport of [761, 900, 1251, 1280, 1440, 1920]) {
        const layout = visiblePanelWidths(viewport, saved, true);
        assert.ok(layout.sidebar >= 220 && layout.sidebar <= 360);
        assert.ok(layout.inspector >= 320 && layout.inspector <= 480);
        const available = viewport - 56 - layout.sidebar - (viewport > 1250 ? layout.inspector : 0);
        assert.ok(available >= 420, `${viewport}: ${available}`);
    }
    assert.deepEqual(saved, {sidebar: 360, inspector: 480});
    assert.equal(visiblePanelWidths(1920, saved, true).inspector, 480);
});

test('inspector uses prototype minimum and compact-desktop cap without changing preferences', () => {
    assert.equal(panelWidth('inspector', 100), 320);
    assert.ok(visiblePanelWidths(1000, {sidebar: 220, inspector: 480}, true).inspector <= 380);
    assert.equal(visiblePanelWidths(1920, {sidebar: 255, inspector: 480}, true).inspector, 480);
});
