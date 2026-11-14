export const sourcePolicies = ['conversationOptIn', 'askWhenRelevant', 'defaultEnabled', 'blocked'] as const;
export const sourceKinds = ['localFile', 'http'] as const;
export type DataSourceContract = {
    sourceId: string; readerDescription: string; schemaFields: string[]; redactedSample: string;
    recordCount: number; maximumAgeMinutes: number; validationState: string; validatedAt: string;
    errorMessage?: string; readRecipe?: unknown;
};
export type DataSource = {
    id: string; name: string; kind: string; location: string; accessPolicy: string;
    validationContract?: DataSourceContract | null; lastError?: string; updatedAt?: string;
};
export type SourceUsage = {
    id: string; sourceId: string; sourceTitle: string; consumer: string; purpose: string;
    conversationId?: string; usedAt: string; disclosedToCloud?: boolean | null;
};
export type SourceDraft = { name: string; kind: string; location: string; accessPolicy: string; maximumAgeMinutes: string };
export class DataSourceError extends Error {
    key: string;
    constructor(key: string) { super(key); this.key = key; }
}
export {randomId as newSourceId} from './randomId.ts';
function object(value: unknown): Record<string, unknown> {
    if (!value || typeof value !== 'object' || Array.isArray(value)) throw new DataSourceError('data_sources.error.response');
    return value as Record<string, unknown>;
}
export function parseSource(value: unknown): DataSource {
    const source = object(value);
    if (!['id', 'name', 'kind', 'location', 'accessPolicy'].every(key => typeof source[key] === 'string') || !source.id) throw new DataSourceError('data_sources.error.response');
    if (source.validationContract != null) {
        const contract = object(source.validationContract);
        if (contract.sourceId !== source.id || !['readerDescription', 'redactedSample', 'validationState', 'validatedAt'].every(key => typeof contract[key] === 'string')
            || !Array.isArray(contract.schemaFields) || !contract.schemaFields.every(value => typeof value === 'string')
            || !Number.isSafeInteger(contract.recordCount) || Number(contract.recordCount) < 0
            || !Number.isSafeInteger(contract.maximumAgeMinutes) || Number(contract.maximumAgeMinutes) < 1) throw new DataSourceError('data_sources.error.response');
    }
    return structuredClone(source) as DataSource;
}
export function parseSources(value: unknown): DataSource[] {
    const rows = object(value).sources;
    if (!Array.isArray(rows)) throw new DataSourceError('data_sources.error.response');
    const sources = rows.map(row => parseSource(object(row).source));
    if (new Set(sources.map(source => source.id)).size !== sources.length) throw new DataSourceError('data_sources.error.response');
    return sources;
}
export function parseUsage(value: unknown): SourceUsage[] {
    const rows = object(value).usageRecords;
    if (!Array.isArray(rows)) throw new DataSourceError('data_sources.error.response');
    return rows.map(row => {
        const record = object(row);
        if (!['id', 'sourceId', 'sourceTitle', 'consumer', 'purpose', 'usedAt'].every(key => typeof record[key] === 'string')
            || record.disclosedToCloud != null && typeof record.disclosedToCloud !== 'boolean'
            || record.conversationId != null && typeof record.conversationId !== 'string') throw new DataSourceError('data_sources.error.response');
        return structuredClone(record) as SourceUsage;
    });
}
export function sourceDraft(source?: DataSource): SourceDraft {
    return { name: source?.name ?? '', kind: source?.kind ?? 'localFile', location: source?.location ?? '',
        accessPolicy: source?.accessPolicy ?? 'conversationOptIn', maximumAgeMinutes: String(source?.validationContract?.maximumAgeMinutes ?? 60) };
}
export function sourcePayload(draft: SourceDraft) {
    if (!draft.name.trim() || !draft.location.trim()) throw new DataSourceError('data_sources.error.required_fields');
    if (!sourceKinds.includes(draft.kind as typeof sourceKinds[number]) || !sourcePolicies.includes(draft.accessPolicy as typeof sourcePolicies[number])) throw new DataSourceError('data_sources.error.unsupported');
    const age = Number(draft.maximumAgeMinutes);
    if (!Number.isSafeInteger(age) || age < 1 || age > 10080) throw new DataSourceError('data_sources.error.invalid_freshness');
    return { name: draft.name.trim(), kind: draft.kind, location: draft.location.trim(), accessPolicy: draft.accessPolicy, maximumAgeMinutes: age };
}
export function sourceReceipt(value: unknown, expectedId?: string): DataSource {
    const receipt = object(value);
    const source = parseSource(receipt.source);
    if (receipt.tested !== true || expectedId != null && source.id !== expectedId) throw new DataSourceError('data_sources.error.response');
    return source;
}
export function deletionReceipt(value: unknown, id: string) {
    const receipt = object(value);
    if (receipt.deleted !== true || receipt.id !== id) throw new DataSourceError('data_sources.error.response');
}
export function sourceState(source: DataSource): string {
    if (source.accessPolicy === 'blocked') return 'blocked';
    if (source.lastError || source.validationContract?.validationState === 'failed') return 'failed';
    if (source.validationContract?.validationState === 'verifiedEmpty') return 'empty';
    if (source.validationContract?.validationState === 'verified') return 'verified';
    return 'not_tested';
}
export function sourceOverview(sources: readonly DataSource[]) {
    const verified = (source: DataSource) => ['verified', 'empty'].includes(sourceState(source))
        && sourcePolicies.includes(source.accessPolicy as typeof sourcePolicies[number]);
    const attention = sources.filter(source => !verified(source));
    return {connected: sources.length, ready: sources.filter(source => source.accessPolicy === 'defaultEnabled' && verified(source)).length, attention};
}
export function disclosureKey(record: SourceUsage): string {
    return record.disclosedToCloud === true ? 'data_sources.usage.cloud' : record.disclosedToCloud === false ? 'data_sources.usage.local' : 'data_sources.usage.unknown';
}
export function replaceSource(sources: DataSource[], receipt: DataSource): DataSource[] {
    return sources.some(source => source.id === receipt.id)
        ? sources.map(source => source.id === receipt.id ? receipt : source) : [...sources, receipt];
}
