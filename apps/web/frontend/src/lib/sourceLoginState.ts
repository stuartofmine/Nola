export type SourceLoginHandle = {request_id: string; account_id: string; expected_revision: number | null};
export type SourceLoginSnapshot = {request_id: string; state: string; authorization_url: string | null;
    inference_supported: false; account: {revision: number; account: {id: string}} | null};

/** A source login is account management, never inference authorization. */
export function sourceLoginSnapshot(raw: unknown, handle: SourceLoginHandle): SourceLoginSnapshot {
    if (!raw || typeof raw !== 'object') throw new Error('Invalid source login response');
    const value = raw as Record<string, unknown>;
    if (value.request_id !== handle.request_id || value.inference_supported !== false ||
        !['starting', 'pending', 'authenticated_source_only', 'failed', 'cancelled'].includes(String(value.state)))
        throw new Error('Source login owner or state changed');
    let authorization_url: string | null = null;
    if (value.authorization_url !== null) {
        if (typeof value.authorization_url !== 'string' || value.state !== 'pending')
            throw new Error('Unexpected source authorization URL');
        const url = new URL(value.authorization_url);
        if (url.protocol !== 'https:' || !['auth.openai.com', 'chatgpt.com'].includes(url.hostname) ||
            url.username || url.password || url.port) throw new Error('Unexpected source authorization origin');
        authorization_url = url.href;
    }
    let account: SourceLoginSnapshot['account'] = null;
    if (value.account !== null) {
        const candidate = value.account as {revision?: unknown; account?: {id?: unknown}} | undefined;
        if (!candidate || candidate.account?.id !== handle.account_id ||
            !Number.isSafeInteger(candidate.revision) || Number(candidate.revision) <= (handle.expected_revision ?? 0))
            throw new Error('Source account receipt changed');
        account = {revision: Number(candidate.revision), account: {id: handle.account_id}};
    }
    if (value.state === 'authenticated_source_only' && !account) throw new Error('Source login completion has no account receipt');
    return {request_id: handle.request_id, state: String(value.state), authorization_url,
        inference_supported: false, account};
}
