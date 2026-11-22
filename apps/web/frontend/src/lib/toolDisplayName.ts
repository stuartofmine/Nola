// SPDX-License-Identifier: Apache-2.0
import catalog from '../../../../../shared/localization/tool-names.json' with { type: 'json' };
import type { Locale } from './i18n';

export function toolDisplayName(language: Locale, capability: unknown, title?: unknown): string {
    const id = typeof capability === 'string' ? capability.trim() : '';
    const declared = typeof title === 'string' ? title.trim() : '';
    const key = (id || declared).toLowerCase().replaceAll('-', '_');
    const aliases: Record<string, string> = catalog.aliases;
    const labels: Record<string, Record<string, string>> = catalog.labels;
    const resolved = Object.hasOwn(aliases, key) ? aliases[key] : key;
    if (!Object.hasOwn(labels, resolved)) return declared || id;
    const names = labels[resolved];
    const titleKey = declared.toLowerCase().replaceAll(' ', '_').replaceAll('-', '_');
    const titleId = Object.hasOwn(aliases, titleKey) ? aliases[titleKey] : titleKey;
    if (declared && declared !== 'Tool' && titleId !== resolved
        && !Object.values(names).includes(declared)) return declared;
    return names[language] ?? names.en;
}
