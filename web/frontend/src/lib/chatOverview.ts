import { contextUsageFraction } from './runContextUsage.ts';
import type { CommittedRunContextUsage } from './runContextUsage.ts';

export interface ChatContextOverview { usedTokens: string; contextWindowTokens: string; fraction: number | null; estimated: boolean }
/** Presentation only; caller supplies the exact already-validated committed record. */
export function committedContextOverview(record: CommittedRunContextUsage | null): ChatContextOverview | null {
    return record ? { usedTokens: record.usage.used_tokens, contextWindowTokens: record.usage.context_window_tokens,
        fraction: contextUsageFraction(record), estimated: record.usage.estimated } : null;
}
