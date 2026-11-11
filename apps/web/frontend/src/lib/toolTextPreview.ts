/** A bounded inline preview. The immutable reader retains the complete field. */
export function toolTextPreview(text: string, maxCharacters = 2000, maxLines = 18): {text: string; truncated: boolean} {
    let end = Math.min(text.length, maxCharacters);
    let lines = 1;
    for (let index = 0; index < end; index++) {
        if (text[index] === '\n' && ++lines > maxLines) { end = index; break; }
    }
    // Do not split a UTF-16 surrogate pair at the character limit.
    if (end > 0 && end < text.length && /[\uD800-\uDBFF]/.test(text[end - 1])) end--;
    const truncated = end < text.length;
    return {text: text.slice(0, end) + (truncated ? '…' : ''), truncated};
}
