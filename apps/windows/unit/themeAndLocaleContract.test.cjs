// SPDX-License-Identifier: Apache-2.0
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { LIGHT_THEME_TOKENS, DARK_THEME_TOKENS } = require('../dist/themeTokens.js');
const { stringsFor } = require('../dist/i18n.js');

const repoRoot = path.resolve(__dirname, '../../..');
const tokens = JSON.parse(fs.readFileSync(path.join(repoRoot, 'shared/design-tokens/tokens.json'), 'utf8'));

const CANONICAL_ROLES = [
    'canvas', 'surface', 'surfaceSubtle', 'surfaceElevated',
    'ink', 'inkSecondary', 'inkMuted',
    'border', 'outline', 'accent', 'onAccent', 'accentSubtle',
    'success', 'warning', 'danger', 'info',
];

function luminance(hex) {
    const channels = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255);
    const linear = channels.map((c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
    return 0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2];
}

function contrastRatio(a, b) {
    const la = luminance(a);
    const lb = luminance(b);
    return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

test('themeTokens.ts defines all 16 canonical Light and Dark roles matching shared tokens.json', () => {
    for (const [label, actual, expected] of [
        ['LIGHT_THEME_TOKENS', LIGHT_THEME_TOKENS, tokens.color.light],
        ['DARK_THEME_TOKENS', DARK_THEME_TOKENS, tokens.color.dark],
    ]) {
        assert.deepEqual(Object.keys(actual).sort(), [...CANONICAL_ROLES].sort(), `${label} role keys`);
        for (const role of CANONICAL_ROLES) {
            assert.equal(actual[role].toLowerCase(), expected[role].toLowerCase(), `${label}.${role}`);
        }
        for (const fg of ['ink', 'inkSecondary', 'inkMuted', 'accent', 'success', 'warning', 'danger', 'info']) {
            for (const bg of ['canvas', 'surface', 'surfaceSubtle', 'surfaceElevated']) {
                assert.ok(contrastRatio(actual[fg], actual[bg]) >= 4.5, `${label} ${fg}/${bg} >= 4.5:1`);
            }
        }
        assert.ok(contrastRatio(actual.onAccent, actual.accent) >= 4.5, `${label} onAccent/accent >= 4.5:1`);
    }
});

test('i18n.ts provides complete non-empty localized dictionaries for all 9 supported locales and BCP-47 prefixes', () => {
    const locales = ['en', 'zh-Hans', 'zh-Hant', 'ja', 'ko', 'de', 'es', 'fr', 'ru'];
    const english = stringsFor('en');
    const keys = Object.keys(english).sort();
    assert.ok(keys.length >= 34);

    const simplifiedMarkers = ['错误', '复制', '路径', '没有', '显示', '窗口', '帮助', '重新载入', '开发者工具', '关闭窗口'];
    for (const locale of locales) {
        const dict = stringsFor(locale);
        assert.deepEqual(Object.keys(dict).sort(), keys, `${locale} key parity`);
        for (const key of keys) {
            assert.equal(typeof dict[key], 'string');
            assert.ok(dict[key].trim().length > 0, `${locale}.${key} must be non-empty`);
            if (locale !== 'en' && key !== 'appName') {
                assert.notEqual(dict[key], english[key], `${locale}.${key} must not fall back to English`);
            }
            if (locale === 'zh-Hant') {
                for (const marker of simplifiedMarkers) {
                    assert.ok(!dict[key].includes(marker), `zh-Hant.${key} leaked simplified Chinese marker ${marker}`);
                }
            }
        }
    }

    const prefixExpectations = [
        ['en-US', 'en'],
        ['zh-CN', 'zh-Hans'],
        ['zh_SG', 'zh-Hans'],
        ['zh-TW', 'zh-Hant'],
        ['zh-HK', 'zh-Hant'],
        ['zh-Hant-TW', 'zh-Hant'],
        ['ja-JP', 'ja'],
        ['ko-KR', 'ko'],
        ['de-DE', 'de'],
        ['es-419', 'es'],
        ['fr-CA', 'fr'],
        ['ru-RU', 'ru'],
        ['unknown-XX', 'en'],
    ];
    for (const [probe, target] of prefixExpectations) {
        assert.deepEqual(stringsFor(probe), stringsFor(target), `${probe} must resolve to ${target}`);
    }
});
