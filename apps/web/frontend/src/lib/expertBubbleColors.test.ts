import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
    expertAccentPalette,
    expertBubbleColor,
    expertBubbleColors,
    expertBubblePalette,
    observeExpertBubbleColors,
    saveExpertBubbleColor,
} from './expertBubbleColors.ts';

test('expert bubble colors assign deterministic fallbacks and notify observers on save', () => {
    const initial = expertBubbleColor('host-a', 'agent-alpha');
    assert.ok(expertBubbleColors.includes(initial));
    assert.ok(expertBubblePalette[initial].length === 2);
    assert.ok(expertAccentPalette[initial].startsWith('#'));

    let notifications = 0;
    const stop = observeExpertBubbleColors(() => {
        notifications++;
    });
    try {
        saveExpertBubbleColor('host-a', 'agent-alpha', 'purple');
        assert.equal(expertBubbleColor('host-a', 'agent-alpha'), 'purple');
        assert.equal(notifications, 1);
    } finally {
        stop();
    }
});
