export type EndpointCredential = {
    endpointID: string;
    baseURL: string;
    wireProtocol: string;
    expectedRevisions: Record<string, number>;
    credentialRevision: number;
    hasCredential: boolean;
};

// Never retain a server object wholesale: the editor only accepts public fields.
export function credentialSnapshot(value: EndpointCredential, expectedID: string): EndpointCredential {
    if (!value || value.endpointID !== expectedID || !expectedID || typeof value.baseURL !== 'string'
        || typeof value.wireProtocol !== 'string' || typeof value.hasCredential !== 'boolean'
        || !Number.isSafeInteger(value.credentialRevision) || value.credentialRevision < 0
        || !value.expectedRevisions || typeof value.expectedRevisions !== 'object' || Array.isArray(value.expectedRevisions)
        || !Object.keys(value.expectedRevisions).length
        || Object.values(value.expectedRevisions).some(revision => !Number.isSafeInteger(revision) || revision < 0)) {
        throw new Error('credential.invalidSnapshot');
    }
    return { endpointID: value.endpointID, baseURL: value.baseURL, wireProtocol: value.wireProtocol,
        expectedRevisions: { ...value.expectedRevisions }, credentialRevision: value.credentialRevision, hasCredential: value.hasCredential };
}

export function isValidCredentialToken(secret: string): boolean {
    return /^[\x21-\x7e]{1,8192}$/.test(secret.trim());
}

export function credentialRequest(snapshot: EndpointCredential, action: 'replace' | 'clear', secret = '') {
    const baseline = credentialSnapshot(snapshot, snapshot.endpointID);
    if (action !== 'replace' && action !== 'clear') throw new Error('credential.invalidAction');
    if (action === 'replace' && !isValidCredentialToken(secret)) throw new Error('credential.invalidToken');
    if (action === 'clear' && secret) throw new Error('credential.conflictingAction');
    return { expectedRevisions: baseline.expectedRevisions, baseURL: baseline.baseURL,
        wireProtocol: baseline.wireProtocol, credentialRevision: baseline.credentialRevision, action,
        ...(action === 'replace' ? { secret: secret.trim() } : {}) };
}

export function credentialPath(id: string) {
    if (!id) throw new Error('credential.missingEndpoint');
    return `/api/model-endpoints/${encodeURIComponent(id)}/credential`;
}
