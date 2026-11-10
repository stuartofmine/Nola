#!/usr/bin/env node
// SPDX-License-Identifier: Apache-2.0
// Validates user-visible macOS data-center and conversation-source copy
// without launching the signed app test host.

const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const catalogPath = path.join(root, "NolaMac", "Localizable.xcstrings");
const requiredLocales = ["en", "zh-Hans", "zh-Hant", "ja", "ko", "fr", "de", "es"];
const criticalKeyPattern = /Data Center|Data source|Sources|Usage|Health|Web source|Default|conversation|attention|validation|Validated|Never used|Last used|^onboarding\.|^mac\.data_center\.permission\./i;

function placeholders(value) {
    return [...String(value).matchAll(/%(?:(\d+)\$)?[-+#0 ']*\d*(?:\.\d+)?(?:hh|h|ll|l|j|z|t|L)?[@a-zA-Z%]/g)]
        .map(match => match[0])
        .filter(token => token !== "%%")
        .sort();
}

function localizedValue(entry, locale, sourceLanguage, key) {
    if (locale === sourceLanguage) {
        return entry.localizations?.[locale]?.stringUnit?.value ?? key;
    }
    return entry.localizations?.[locale]?.stringUnit?.value ?? "";
}

const catalog = JSON.parse(fs.readFileSync(catalogPath, "utf8"));
const failures = [];
let checkedKeys = 0;

for (const [key, entry] of Object.entries(catalog.strings ?? {})) {
    if (!criticalKeyPattern.test(key) || entry.shouldTranslate === false) continue;
    checkedKeys += 1;
    const sourceValue = localizedValue(entry, catalog.sourceLanguage, catalog.sourceLanguage, key);
    const sourcePlaceholders = placeholders(sourceValue);

    for (const locale of requiredLocales) {
        const value = localizedValue(entry, locale, catalog.sourceLanguage, key);
        if (!value.trim()) {
            failures.push(`${key}: missing ${locale}`);
            continue;
        }
        const localizedPlaceholders = placeholders(value);
        if (JSON.stringify(localizedPlaceholders) !== JSON.stringify(sourcePlaceholders)) {
            failures.push(
                `${key}: ${locale} placeholders ${JSON.stringify(localizedPlaceholders)} do not match ${JSON.stringify(sourcePlaceholders)}`
            );
        }
    }
}

if (checkedKeys === 0) failures.push("No critical localization keys matched the validation scope.");

if (failures.length > 0) {
    console.error(`macOS localization validation failed (${failures.length} issue${failures.length === 1 ? "" : "s"}):`);
    for (const failure of failures) console.error(`- ${failure}`);
    process.exit(1);
}

console.log(`macOS localization validation passed: ${checkedKeys} critical keys, ${requiredLocales.length} locales`);
