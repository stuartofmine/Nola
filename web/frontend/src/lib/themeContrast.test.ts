import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const css = readFileSync(new URL('../theme.css', import.meta.url), 'utf8');
const luminance = (hex: string) => {
    const full = hex.length === 3 ? [...hex].map(value => value.repeat(2)).join('') : hex;
    const channels = full.match(/../g)!.map(value => parseInt(value, 16) / 255)
        .map(value => value <= .04045 ? value / 12.92 : ((value + .055) / 1.055) ** 2.4);
    return channels[0] * .2126 + channels[1] * .7152 + channels[2] * .0722;
};
test('light and dark semantic text retains AA contrast including primary buttons', () => {
    const palette: Record<string, string> = {};
    const blocks = [css.split('@media')[0], css.split('@media (prefers-color-scheme: dark)')[1].split('* {')[0]];
    for (const block of blocks) {
        for (const match of block.matchAll(/--([\w-]+):\s*#([\da-f]+)/gi)) palette[match[1]] = match[2];
        for (const [foreground, background] of [['content', 'canvas'], ['muted', 'panel'], ['accent', 'surface'], ['on-accent', 'accent'], ['danger', 'surface']]) {
            const a = luminance(palette[foreground]), b = luminance(palette[background]);
            assert.ok((Math.max(a, b) + .05) / (Math.min(a, b) + .05) >= 4.5, `${foreground}/${background}`);
        }
    }
});
