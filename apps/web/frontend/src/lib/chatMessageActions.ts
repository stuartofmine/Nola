import { markdownPlainText } from './markdown.ts';
import { timelineMessageText } from './chatTimeline.ts';
import type { TimelineMessage } from './chatTimeline.ts';
import { displayTextAttachmentMessage } from './chatTextAttachments.ts';

/** Copy only disclosed message prose, never hidden reasoning, tool data or input envelopes. */
export function messageCopyContent(message: TimelineMessage, format: 'plain' | 'markdown'): string {
    const source = timelineMessageText(message);
    if (message.role === 'user') return displayTextAttachmentMessage(source)?.displayText ?? source;
    return format === 'markdown' ? source : markdownPlainText(source);
}
/** Only persisted, settled message boundaries may alter conversation history. */
export function messageMutationId(message: TimelineMessage): string | null {
    return message.canonical?.message_id === message.id && message.id.trim()
        && !['pending', 'running'].includes(message.state) ? message.id : null;
}
