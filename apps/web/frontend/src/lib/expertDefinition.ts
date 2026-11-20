import type {PersistentAgent} from './agentRooms.ts';

export interface ExpertExecutorModel { id: string; display_name: string; provider_id?: string | null }
export interface ExpertExecutorOption {
    id: string; kind: string; display_name: string; availability: string; models?: ExpertExecutorModel[];
}

export function expertExecutorSelection(agent: PersistentAgent | null): string {
    const binding = agent?.defaults.expertExecutor as {backend?: string; model?: string; model_provider?: string} | undefined;
    return binding?.backend ? JSON.stringify([binding.backend, binding.model ?? '', binding.model_provider ?? '']) : '';
}

export function expertExecutorKey(backend: string, model?: ExpertExecutorModel): string {
    return JSON.stringify([backend, model?.id ?? '', backend === 'codex' ? model?.provider_id ?? '' : '']);
}

/** Updating two explicit choices must preserve every other Core-owned default. */
export function editedExpertDefaults(current: PersistentAgent | null, model: string, template: string, effort?: string, executor?: string): Record<string, unknown> {
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
    if (executor) delete defaults.expertReasoningEffort;
    if (executor !== undefined && executor !== expertExecutorSelection(current)) {
        if (!executor) delete defaults.expertExecutor;
        else {
            const [backend, model, provider] = JSON.parse(executor) as string[];
            defaults.expertExecutor = {backend, ...(model && {model}), ...(provider && {model_provider: provider})};
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

export type ExpertEditorValues = [string, string, string, string, string, boolean, string];
export function expertEditorValues(agent: PersistentAgent): ExpertEditorValues {
    return [agent.name,agent.description,typeof agent.defaults.modelSelectionID === 'string' ? agent.defaults.modelSelectionID : '',
        typeof agent.defaults.selectedPromptTemplateID === 'string' ? agent.defaults.selectedPromptTemplateID : '',
        typeof agent.defaults.expertReasoningEffort === 'string' ? agent.defaults.expertReasoningEffort : 'automatic',agent.availability === 'active', expertExecutorSelection(agent)];
}
/** Refresh carries forward only actual draft changes, preserving concurrent edits to untouched controls. */
export function rebaseExpertEditorValues(draft: ExpertEditorValues, baseline: ExpertEditorValues, latest: ExpertEditorValues): ExpertEditorValues {
    return draft.map((value,index) => value === baseline[index] ? latest[index] : value) as ExpertEditorValues;
}
