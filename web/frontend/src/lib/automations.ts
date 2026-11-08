import { executionConversationLink } from './results.ts';
export const automationFormats = ['agent', 'html', 'search', 'rss', 'json', 'icalendar', 'file', 'folder', 'script'] as const;
export type AutomationFormat = typeof automationFormats[number];
export interface Automation {
    executionSpec?: unknown;
    id: string; name: string; url: string; format: string; userPrompt: string; revision?: number;
    frequency: string; weekday: number | null; time: string; timezone: string;
    enabled: boolean; updatedAt: string; scriptApproved: boolean | null;
    scriptCwd: string | null; latestResult: { resultId: string; status: string; createdAt: string } | null;
}
export interface AutomationDraft {
    skillIds: string[];
    workspaceRoot: string; permissionProfile: string;
    name: string; format: string; source: string; prompt: string; script: string; cwd: string;
    frequency: string; weekday: number; time: string;
}
export interface AutomationCapabilities { manualRun: boolean; scheduledDispatch: boolean; promptAutomations: boolean }
export function automationCapabilities(value: unknown): AutomationCapabilities {
    const capabilities = (value as { executionCapabilities?: Record<string, unknown> })?.executionCapabilities;
    return { manualRun: capabilities?.manualRun === true, scheduledDispatch: capabilities?.scheduledDispatch === true, promptAutomations: capabilities?.promptAutomations === true };
}
export function automationCanApprove(action: Automation): boolean { return !!automationScript(action) && Number.isSafeInteger(action.revision) && action.revision! > 0; }
export function newAutomationDraft(): AutomationDraft {
    return { skillIds: [], workspaceRoot: '', permissionProfile: '', name: '', format: 'agent', source: '', prompt: '', script: '', cwd: '', frequency: 'daily', weekday: 1, time: '09:00' };
}
export function parseAutomations(value: unknown): Automation[] {
    const items = (value as { actions?: unknown })?.actions;
    if (!Array.isArray(items)) throw new Error('automation.invalidResponse');
    const ids = new Set<string>();
    return items.map(item => {
        if (!item || ['id', 'name', 'url', 'format', 'userPrompt', 'frequency', 'time', 'timezone', 'updatedAt'].some(key => typeof item[key] !== 'string') || !item.id || typeof item.enabled !== 'boolean' || ids.has(item.id)) throw new Error('automation.invalidResponse');
        ids.add(item.id);
        return { ...item, scriptApproved: item.scriptApproved === true ? true : item.scriptApproved === false ? false : null } as Automation;
    });
}
export function automationCreateRequest(draft: AutomationDraft): Record<string, unknown> {
    if (!Array.isArray(draft.skillIds) || draft.skillIds.some(id => typeof id !== 'string' || !id.trim()) || new Set(draft.skillIds).size !== draft.skillIds.length) throw new Error('automation.invalidDraft');
    if (!draft.workspaceRoot.trim().startsWith('/') || /[\u0000-\u001f]/.test(draft.workspaceRoot) || !draft.prompt.trim() || !['cautious', 'allow_all'].includes(draft.permissionProfile)) throw new Error('automation.invalidDraft');
    if (!automationFormats.includes(draft.format as AutomationFormat) || !['daily', 'weekly'].includes(draft.frequency) || !/^([01]\d|2[0-3]):[0-5]\d$/.test(draft.time) || (draft.frequency === 'weekly' && (!Number.isInteger(draft.weekday) || draft.weekday < 1 || draft.weekday > 7)) || !draft.name.trim()) throw new Error('automation.invalidDraft');
    const body: Record<string, unknown> = { name: draft.name.trim(), format: draft.format, userPrompt: draft.prompt, frequency: draft.frequency, time: draft.time, timezone: 'local' };
    body.executionSpec = { kind: 'agent', spec: { prompt: draft.prompt,
        execution_scope: { kind: 'local_workspace', root: draft.workspaceRoot.trim() },
        permission_profile: draft.permissionProfile, model_selection: 'use_current_default_at_admission',
        context_refs: [], skill_refs: [...draft.skillIds] } };
    if (draft.frequency === 'weekly') body.weekday = draft.weekday;
    if (draft.format === 'script') {
        if (!draft.script.trim() || draft.script.includes('\0')) throw new Error('automation.invalidDraft');
        body.script = draft.script; body.cwd = draft.cwd.trim();
    } else if (draft.format !== 'agent') {
        if (!draft.source.trim()) throw new Error('automation.invalidDraft');
        body[draft.format === 'search' ? 'query' : ['file', 'folder'].includes(draft.format) ? 'path' : 'url'] = draft.source.trim();
    }
    return body;
}
export function automationScript(action: Automation): { script: string; cwd: string } | null {
    if (action.format !== 'script') return null;
    try {
        const value = JSON.parse(action.userPrompt);
        if (value.kind !== 'script_automation' || value.runtime !== 'bash' || typeof value.script !== 'string' || !value.script.trim() || typeof value.cwd !== 'string') return null;
        return { script: value.script, cwd: value.cwd };
    } catch { return null; }
}
export function automationEditDraft(action: Automation): AutomationDraft {
    const execution = automationExecutionSpec(action);
    const spec = (action.executionSpec as { spec?: Record<string, unknown> })?.spec;
    if (!execution || !automationKnown(action) || !Number.isSafeInteger(action.revision) || action.revision! < 1
        || action.timezone !== 'local' || spec?.model_selection !== 'use_current_default_at_admission'
        || !Array.isArray(spec.context_refs) || spec.context_refs.length || !Array.isArray(spec.skill_refs)) throw new Error('automation.invalidDraft');
    const script = automationScript(action);
    if (action.format === 'script' && !script) throw new Error('automation.scriptUnavailable');
    const draft = { ...newAutomationDraft(), name: action.name, format: action.format, source: action.url,
        prompt: execution.prompt, workspaceRoot: execution.root, permissionProfile: execution.permission,
        script: script?.script ?? '', cwd: script?.cwd ?? '', frequency: action.frequency, weekday: action.weekday ?? 1, time: action.time, skillIds: [...spec.skill_refs] as string[] };
    automationCreateRequest(draft);
    return draft;
}
export function automationCanEdit(action: Automation): boolean {
    try { automationEditDraft(action); return true; } catch { return false; }
}
export interface AutomationSkill { id: string; name: string; description: string }
export function parseAutomationSkills(value: unknown): AutomationSkill[] {
    const skills = (value as { skills?: unknown })?.skills;
    if (!Array.isArray(skills)) throw new Error('automation.invalidResponse');
    const ids = new Set<string>();
    return skills.map(item => {
        if (!item || typeof item.id !== 'string' || !item.id.trim() || ids.has(item.id) || typeof item.name !== 'string' || typeof item.description !== 'string') throw new Error('automation.invalidResponse');
        ids.add(item.id);
        return { id: item.id, name: item.name, description: item.description };
    });
}
export function automationEditRequest(action: Automation, draft: AutomationDraft): Record<string, unknown> {
    automationEditDraft(action);
    return { expectedRevision: action.revision, definition: automationCreateRequest(draft) };
}
export function automationApprovalRequest(action: Automation): Record<string, unknown> {
    if (!automationCanApprove(action)) throw new Error('automation.scriptUnavailable');
    return { confirmSafety: true, expectedRevision: action.revision };
}
export function sameAutomationRevision(a: Automation, b: Automation | undefined): boolean {
    return !!b && a.id === b.id && a.revision === b.revision && a.enabled === b.enabled && a.userPrompt === b.userPrompt && a.updatedAt === b.updatedAt && a.format === b.format && JSON.stringify(a.executionSpec) === JSON.stringify(b.executionSpec);
}
export function automationExecutionSpec(action: Automation): { prompt: string; root: string; permission: string } | null {
    const value = action.executionSpec as { kind?: unknown; spec?: { prompt?: unknown; execution_scope?: { kind?: unknown; root?: unknown }; permission_profile?: unknown } } | undefined;
    const spec = value?.spec;
    if (value?.kind !== 'agent' || typeof spec?.prompt !== 'string' || spec.execution_scope?.kind !== 'local_workspace' || typeof spec.execution_scope.root !== 'string' || typeof spec.permission_profile !== 'string' || !['cautious', 'allow_all'].includes(spec.permission_profile)) return null;
    return { prompt: spec.prompt, root: spec.execution_scope.root, permission: spec.permission_profile };
}
export function automationKnown(action: Automation): boolean { return automationFormats.includes(action.format as AutomationFormat); }
export function automationCanRun(action: Automation): boolean { return automationKnown(action) && (action.format !== 'script' || action.scriptApproved === true); }
export function automationHistoryLink(action: Automation): string { return `/results?sourceId=${encodeURIComponent(action.id)}`; }
export function automationWeekday(day: number, locale: string): string {
    if (!Number.isInteger(day) || day < 1 || day > 7) return String(day);
    // Host contract follows chrono Sunday=1, not ISO Monday=1.
    return new Intl.DateTimeFormat(locale, { weekday: 'long', timeZone: 'UTC' }).format(new Date(Date.UTC(2023, 0, day)));
}

export interface AutomationRunIntent { definitionId: string; expectedRevision: number; occurrenceKey: string }
export function automationRunConversationLink(value: unknown, expectedRevision: number): string | undefined {
    const data = value as { occurrenceId?: unknown; execution?: { binding?: Record<string, unknown> }; receipt?: { binding?: Record<string, unknown> } };
    const binding = data?.receipt?.binding ?? data?.execution?.binding;
    if (!binding || binding.occurrence_id !== data.occurrenceId || binding.definition_revision !== expectedRevision) return;
    return executionConversationLink(binding.conversation_id, binding.run_id);
}
export function automationRunIntent(action: Automation, previous?: AutomationRunIntent): AutomationRunIntent {
    if (!Number.isSafeInteger(action.revision) || action.revision! <= 0) throw new Error('automation.approvalChanged');
    if (previous) {
        if (previous.definitionId !== action.id || previous.expectedRevision !== action.revision || !previous.occurrenceKey.trim()) throw new Error('automation.approvalChanged');
        return previous;
    }
    return { definitionId: action.id, expectedRevision: action.revision!, occurrenceKey: `web-manual-${crypto.randomUUID()}` };
}
export function automationRunResponse(value: unknown, expectedRevision: number): { terminal: boolean; message: string } {
    const data = value as { occurrenceId?: unknown; execution?: unknown; receipt?: { binding?: { occurrence_id?: unknown; definition_revision?: unknown }; terminal?: { result?: { value?: { kind?: unknown } } } }; outcomeUnknown?: unknown };
    if (!data || typeof data.occurrenceId !== 'string' || !data.occurrenceId || !data.execution) throw new Error('automation.invalidResponse');
    if (!data.receipt) return { terminal: false, message: data.outcomeUnknown === true ? 'automation.runUnknown' : 'automation.runAccepted' };
    if (data.receipt.binding?.occurrence_id !== data.occurrenceId || data.receipt.binding?.definition_revision !== expectedRevision) throw new Error('automation.invalidResponse');
    const kind = data.receipt.terminal?.result?.value?.kind;
    if (typeof kind !== 'string' || !['answered', 'action_completed', 'incomplete', 'blocked', 'cancelled', 'failed'].includes(kind)) throw new Error('automation.invalidResponse');
    return { terminal: true, message: `automation.outcome.${kind}` };
}
