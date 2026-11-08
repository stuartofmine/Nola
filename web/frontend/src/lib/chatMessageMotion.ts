/** IDs that were appended after an already-rendered timeline snapshot.
 * Initial hydration, replacement, and history prepends intentionally return none. */
export function appendedMessageIds(previous: readonly string[], current: readonly string[], historyPrepending = false, allowEmptyAppend = false): string[] {
    if (historyPrepending || current.length <= previous.length || previous.length === 0 && !allowEmptyAppend) return [];
    for (let index = 0; index < previous.length; index++) {
        if (previous[index] !== current[index]) return [];
    }
    return current.slice(previous.length);
}
