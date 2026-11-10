export interface ConversationPdfResource {
    resourceId: string;
    conversationId: string;
    displayName: string;
    mimeType: 'application/pdf';
    byteCount: number;
    sha256: string;
}

const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const sha256 = /^[0-9a-f]{64}$/i;
export const MAX_PDF_BYTES = 8 * 1024 * 1024;

export function conversationResourcesUrl(conversationId: string): string {
    if (!conversationId || /[\p{Cc}]/u.test(conversationId)) throw new Error('chatResource.invalidConversation');
    return `/api/chat/conversations/${encodeURIComponent(conversationId)}/resources`;
}

export function conversationPdfUrl(conversationId: string, resourceId: string): string {
    if (!uuid.test(resourceId)) throw new Error('chatResource.invalidResource');
    return `${conversationResourcesUrl(conversationId)}/${encodeURIComponent(resourceId)}`;
}

export function parseConversationPdfResource(value: unknown, conversationId: string): ConversationPdfResource {
    if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('chatResource.invalidResponse');
    const item = value as Record<string, unknown>;
    if (!uuid.test(String(item.resourceId ?? '')) || item.conversationId !== conversationId
        || typeof item.displayName !== 'string' || !item.displayName.trim() || item.displayName.length > 256
        || item.mimeType !== 'application/pdf' || !Number.isSafeInteger(item.byteCount)
        || Number(item.byteCount) < 1 || Number(item.byteCount) > MAX_PDF_BYTES
        || typeof item.sha256 !== 'string' || !sha256.test(item.sha256)
        || item.previewUrl !== conversationPdfUrl(conversationId, item.resourceId as string)) throw new Error('chatResource.invalidResponse');
    return { resourceId: item.resourceId as string, conversationId, displayName: item.displayName,
        mimeType: 'application/pdf', byteCount: item.byteCount as number, sha256: item.sha256 };
}

export function parseConversationPdfResources(value: unknown, conversationId: string): ConversationPdfResource[] {
    if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('chatResource.invalidResponse');
    const payload = value as Record<string, unknown>;
    if (payload.conversationId !== conversationId || !Array.isArray(payload.resources)) throw new Error('chatResource.invalidResponse');
    const resources = payload.resources.map(item => parseConversationPdfResource(item, conversationId));
    if (new Set(resources.map(item => item.resourceId)).size !== resources.length) throw new Error('chatResource.invalidResponse');
    return resources;
}

export function validatePdfFile(file: Pick<File, 'name' | 'size' | 'type'>): void {
    if (!file.name.trim() || file.name.length > 256 || file.size < 1 || file.size > MAX_PDF_BYTES
        || !file.name.toLowerCase().endsWith('.pdf') || (file.type !== '' && file.type !== 'application/pdf')) throw new Error('chatResource.invalidFile');
}

export async function pdfFileBase64(file: File): Promise<string> {
    validatePdfFile(file);
    const bytes = new Uint8Array(await file.arrayBuffer());
    if (bytes.length !== file.size) throw new Error('chatResource.invalidFile');
    // Chunking avoids spreading an 8 MiB array onto the JS call stack.
    let binary = '';
    for (let offset = 0; offset < bytes.length; offset += 8192) binary += String.fromCharCode(...bytes.subarray(offset, offset + 8192));
    return btoa(binary);
}
