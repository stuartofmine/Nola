// SPDX-License-Identifier: Apache-2.0
// Shared by legacy chat and the Svelte transport. No DOM or permission inference.

/** @typedef {'invalid_frame'|'incomplete_frame'|'frame_too_large'|'invalid_utf8'|'invalid_content_type'|'missing_body'} TransportCode */
export class ChatTransportError extends Error {
    /** @param {TransportCode} code */
    constructor(code) {
        super(`chat.transport.${code}`);
        this.code = code;
    }
}

/** Framing only: neither EOF nor [DONE] is a durable conversation commit.
 * @param {number} [maxFrameCharacters]
 */
export function createSseJsonDecoder(maxFrameCharacters = 16 * 1024 * 1024) {
    if (!Number.isSafeInteger(maxFrameCharacters) || maxFrameCharacters < 1) throw new RangeError('Invalid frame limit');
    let line = '', skipLF = false, first = true, size = 0, closed = false;
    /** @type {string[]} */
    let data = [];
    /** @param {Record<string, unknown>[]} output */
    function consumeLine(output) {
        if (line === '') {
            size = 0;
            if (data.length) {
                const payload = data.join('\n'); data = [];
                if (payload !== '[DONE]') {
                    let value;
                    try { value = JSON.parse(payload); } catch { throw new ChatTransportError('invalid_frame'); }
                    if (!value || typeof value !== 'object' || Array.isArray(value)) throw new ChatTransportError('invalid_frame');
                    output.push(value);
                }
            }
        } else if (line === 'data') data.push('');
        else if (line.startsWith('data:')) data.push(line.slice(5).replace(/^ /, ''));
        line = '';
    }
    return {
        /** @param {string} chunk @returns {Record<string, unknown>[]} */
        push(chunk) {
            if (closed) throw new ChatTransportError('invalid_frame');
            /** @type {Record<string, unknown>[]} */
            const output = [];
            try {
                for (const character of chunk) {
                    if (first) { first = false; if (character === '\uFEFF') continue; }
                    if (skipLF) { skipLF = false; if (character === '\n') continue; }
                    if (++size > maxFrameCharacters) throw new ChatTransportError('frame_too_large');
                    if (character === '\r' || character === '\n') {
                        consumeLine(output); skipLF = character === '\r';
                    } else line += character;
                }
            } catch (error) { closed = true; line = ''; data = []; throw error; }
            return output;
        },
        /** @returns {Record<string, unknown>[]} */
        finish() {
            if (closed) throw new ChatTransportError('invalid_frame');
            closed = true;
            // Never invent a frame boundary at EOF or accept a truncated commit.
            if (data.length || line === 'data' || line.startsWith('data:')) throw new ChatTransportError('incomplete_frame');
            return [];
        },
    };
}
