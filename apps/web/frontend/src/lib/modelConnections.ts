import { api } from './api.ts';
export type Connection = {id: string; display_name: string; base_url: string; protocols: string[]; source: string; source_id: string | null; account_id: string | null; enabled: boolean};
export type CatalogModel = {model_id: string; display_name: string; protocol: string; context_window: number | null; modalities: string[]; reasoning_levels: string[]; summary_supported?: boolean | null};
export type ExecutionCapabilities = {toolCalls: boolean; imageInput: boolean; reasoning: boolean; samplingParameters: boolean; samplingRequiresNoReasoning: boolean; chatOutputTokenField: string};
export type CapabilityAssessment = {model_id: string; protocol: string; evidence: {text_generation: boolean; credential_binding: string; account_revision: number | null; observed_at_ms: number; requests_sent: number} | null; user_override: ExecutionCapabilities | null};
export type ConnectionSnapshot = {revision: number; probe_credential_bindings?: Record<string,string>; document: {format_version: number; connection: Connection; catalog: {observed_at_ms: number; models: CatalogModel[]} | null; capability_assessments?: CapabilityAssessment[] | null}};
export type ConnectionInventory = {connections: ConnectionSnapshot[]; accounts: {revision: number; account: {id: string; display_name: string; source: string; state: string}}[];
    selections: {connection_id: string; catalog_key: string; enabled: boolean; selected: boolean}[]};
export type ImportPlan = {plan_id: string; expires_at_ms: number; candidates: {candidate_id: string; connection: Connection; models: CatalogModel[]; credential_status: string; issue: string | null; existing_revision: number | null}[]};
export type ImportResult = {candidate_id: string; connection_id: string | null; status: string; error: string | null};
export const loadConnections = (signal?: AbortSignal) => api<ConnectionInventory>('/api/model-connections', {signal});
export function modelProbeConfirmation(snapshot: ConnectionSnapshot, model: CatalogModel, accountRevision: number | null, requestID: string, acknowledged: boolean) {
    const connection = snapshot.document.connection;
    const binding = snapshot.probe_credential_bindings?.[model.protocol];
    if (!acknowledged || !connection.enabled || !binding || (connection.account_id !== null && accountRevision === null)
        || !snapshot.document.catalog?.models.some(item => item.model_id === model.model_id && item.protocol === model.protocol)) {
        throw new Error('Explicit current account/model/protocol and possible-charge confirmation required');
    }
    return {request_id:requestID, connection_id:connection.id, expected_revision:snapshot.revision,
        account_id:connection.account_id, account_revision:accountRevision, credential_binding:binding,
        model_id:model.model_id, protocol:model.protocol, max_requests:1, acknowledge_possible_charges:true};
}
export function modelSelectionCommand(snapshot: ConnectionSnapshot, inventory: ConnectionInventory, key: string, enabled: boolean) {
    const current = inventory.selections.filter(item => item.connection_id === snapshot.document.connection.id);
    const keys = new Set(current.filter(item => item.enabled).map(item => item.catalog_key));
    if (enabled) keys.add(key); else keys.delete(key);
    const selected = current.find(item => item.selected && keys.has(item.catalog_key))?.catalog_key ?? null;
    return {type: 'configure_connection_models', connection_id: snapshot.document.connection.id, expected_revision: snapshot.revision,
        enabled_model_ids: [...keys], selected_model_id: selected, updated_at_ms: Date.now()};
}
