/** Enter confirms IME candidates before it can submit a conversation draft. */
export function composerEnterSubmits(event: Pick<KeyboardEvent, 'key' | 'shiftKey' | 'isComposing' | 'keyCode' | 'repeat'>, composing: boolean): boolean {
    // Safari can report isComposing=false on the Enter ending composition.
    return event.key === 'Enter' && !event.shiftKey && !event.isComposing
        && !composing && event.keyCode !== 229 && !event.repeat;
}
