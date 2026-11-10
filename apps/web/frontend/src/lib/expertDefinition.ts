import type {PersistentAgent} from './agentRooms.ts';

/** Updating two explicit choices must preserve every other Core-owned default. */
export function editedExpertDefaults(current: PersistentAgent | null, model: string, template: string): Record<string, unknown> {
    const defaults = {...current?.defaults};
    for (const [key, value] of [['modelSelectionID', model], ['selectedPromptTemplateID', template]]) {
        const previous = typeof defaults[key] === 'string' ? defaults[key] : '';
        if (value === previous) continue;
        if (value) defaults[key] = value;
        else delete defaults[key];
    }
    return defaults;
}

function canonical(value: unknown): string {
    if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`;
    if (value !== null && typeof value === 'object') return `{${Object.entries(value).sort(([a], [b]) => a.localeCompare(b))
        .map(([key, entry]) => `${JSON.stringify(key)}:${canonical(entry)}`).join(',')}}`;
    return JSON.stringify(value) ?? 'undefined';
}

/** Presence of the old ID alone cannot confirm an uncertain edit was committed. */
export function confirmsExpertWrite(committed: PersistentAgent, attempted: PersistentAgent): boolean {
    return committed.id === attempted.id && committed.revision > attempted.revision
        && committed.name === attempted.name && committed.description === attempted.description
        && committed.availability === attempted.availability
        && canonical(committed.defaults) === canonical(attempted.defaults);
}
