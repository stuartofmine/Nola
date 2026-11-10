// SPDX-License-Identifier: Apache-2.0
// Run with: node --test scripts/test_siwc_localizations.mjs
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import assert from 'node:assert/strict';

test('native SIWC labels, phases and safe errors cover every Mac locale', () => {
    const catalog = JSON.parse(readFileSync(new URL('../apps/macos/Localizable.xcstrings', import.meta.url), 'utf8'));
    const files = ['UI/MacSiwcSignInView.swift', 'UI/MacSiwcAccountManagementView.swift', 'Services/MacSiwcAccountBridge.swift'];
    const keys = new Set(['Signed in', 'Signed out', 'Reauthorize %@', 'ChatGPT authorization on this Mac',
        'Add ChatGPT Authorization',
        'Authorize Nola independently for this Mac’s account store. This does not reuse Codex login or change a paired Host.',
        'The local runtime owns this authorization. Discover and enable models explicitly; signing in does not itself verify inference.']);
    for (const file of files) {
        const source = readFileSync(new URL('../apps/macos/' + file, import.meta.url), 'utf8');
        for (const match of source.matchAll(/(?:Text|Button|Toggle|TextField|String\(localized:|confirmationDialog)\s*\(?\s*"([^"]+)"/g)) keys.add(match[1]);
        if (file.endsWith('MacSiwcSignInView.swift')) {
            const phases = source.slice(source.indexOf('private func phaseLabel'));
            for (const match of phases.matchAll(/(?:case "[^"]+"|default): "([^"]+)"/g)) keys.add(match[1]);
        }
    }
    for (const key of keys) {
        for (const locale of ['en', 'zh-Hans', 'zh-Hant', 'de', 'es', 'fr', 'ja', 'ko', 'ru']) {
            const unit = catalog.strings[key]?.localizations?.[locale]?.stringUnit;
            assert.equal(unit?.state, 'translated', `${locale}: ${key}`);
            assert.ok(unit.value.trim(), `${locale}: ${key}`);
            assert.equal((unit.value.match(/%@/g) ?? []).length, (key.match(/%@/g) ?? []).length, `${locale}: ${key}`);
            if (locale !== 'en') assert.notEqual(unit.value, key, `${locale} must not silently reuse English: ${key}`);
        }
    }
    assert.ok(keys.size >= 46);
});
