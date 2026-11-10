import { apiResponse } from './api.ts';
import { conversationPdfUrl, MAX_PDF_BYTES, type ConversationPdfResource } from './chatResources.ts';

/** Both inspector and content tabs use the same bounded, same-origin reader. */
export async function loadConversationPdf(resource: ConversationPdfResource, signal: AbortSignal): Promise<Blob> {
    const response = await apiResponse(conversationPdfUrl(resource.conversationId, resource.resourceId),
        {signal, redirect: 'error', cache: 'no-store'});
    if (!response.ok || response.headers.get('Content-Type')?.split(';')[0].trim() !== 'application/pdf') {
        throw new Error('chatResource.invalidResponse');
    }
    const blob = await response.blob();
    if (blob.size !== resource.byteCount || blob.size > MAX_PDF_BYTES) throw new Error('chatResource.invalidResponse');
    return blob;
}
