import { timelineMessageText, type TimelineMessage } from './chatTimeline.ts';

/** Mirrors Mac transcript search: case-insensitive matching over loaded
 * message text, never reasoning, tool arguments, or unloaded history. */
export function matchingChatMessageIds(messages: readonly TimelineMessage[], query: string, locale: string): Set<string> {
    const needle = query.trim().toLocaleLowerCase(locale);
    if (!needle) return new Set();
    return new Set(messages.filter(message => timelineMessageText(message).toLocaleLowerCase(locale).includes(needle)).map(message => message.id));
}
