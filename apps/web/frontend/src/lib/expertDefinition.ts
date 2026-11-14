import type {PersistentAgent} from './agentRooms.ts';

/** Updating two explicit choices must preserve every other Core-owned default. */
export function editedExpertDefaults(current: PersistentAgent | null, model: string, template: string, effort?: string): Record<string, unknown> {
    const defaults = {...current?.defaults};
    for (const [key, value] of [['modelSelectionID', model], ['selectedPromptTemplateID', template]]) {
        const previous = typeof defaults[key] === 'string' ? defaults[key] : '';
        if (value === previous) continue;
        if (value) defaults[key] = value;
        else delete defaults[key];
    }
    if (effort !== undefined) {
        if (!effort || effort === 'automatic') delete defaults.expertReasoningEffort;
        else {
            if (!['none','minimal','low','medium','high','xhigh','max'].includes(effort)) throw new Error('Invalid expert reasoning effort');
            defaults.expertReasoningEffort = effort;
        }
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

export type ExpertEditorValues = [string, string, string, string, string, boolean];
export function expertEditorValues(agent: PersistentAgent): ExpertEditorValues {
    return [agent.name,agent.description,typeof agent.defaults.modelSelectionID === 'string' ? agent.defaults.modelSelectionID : '',
        typeof agent.defaults.selectedPromptTemplateID === 'string' ? agent.defaults.selectedPromptTemplateID : '',
        typeof agent.defaults.expertReasoningEffort === 'string' ? agent.defaults.expertReasoningEffort : 'automatic',agent.availability === 'active'];
}
/** Refresh carries forward only actual draft changes, preserving concurrent edits to untouched controls. */
export function rebaseExpertEditorValues(draft: ExpertEditorValues, baseline: ExpertEditorValues, latest: ExpertEditorValues): ExpertEditorValues {
    return draft.map((value,index) => value === baseline[index] ? latest[index] : value) as ExpertEditorValues;
}
