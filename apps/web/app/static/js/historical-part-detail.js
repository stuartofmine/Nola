// SPDX-License-Identifier: Apache-2.0
// One bounded, read-only detail request. This never mutates the conversation reducer.
(function installHistoricalPartDetail(global) {
    'use strict';
    const states = new Set(['supported', 'deferred', 'unsupported', 'not_loaded']);
    function descriptor(part) {
        const projection = part?.projection;
        const owner = projection?.owner;
        if (part?.kind !== 'historical' || !owner || part.part_id !== owner.part_id
            || !Number.isSafeInteger(projection.canonical_revision) || projection.canonical_revision < 1
            || ![owner.conversation_id, owner.message_id, owner.part_id].every(id => typeof id === 'string' && id.length > 0)) return null;
        return { owner: { ...owner }, revision: projection.canonical_revision,
            fingerprint: JSON.stringify(projection) };
    }
    function matches(result, request) {
        return result?.canonical_revision === request.revision
            && ['conversation_id', 'message_id', 'part_id'].every(key => result?.owner?.[key] === request.owner[key]);
    }
    function validate(result, request) {
        if (!matches(result, request)) throw new Error('Historical detail owner or revision changed');
        if (!states.has(result?.status?.state)) throw new Error('Unknown historical detail status');
        if (result.status.state === 'not_loaded' && result.status.reason !== 'selection_budget') throw new Error('Unknown historical detail completeness');
        if (!result.fields || typeof result.fields !== 'object' || Array.isArray(result.fields)) throw new Error('Invalid historical detail fields');
        for (const field of Object.values(result.fields)) {
            if (!['complete', 'deferred'].includes(field?.availability)) throw new Error('Unknown historical field availability');
        }
        if (new TextEncoder().encode(JSON.stringify(result)).length > 32768) throw new Error('Historical detail exceeds byte budget');
        return result;
    }
    function createLoader({ read, isCurrent, onState }) {
        let generation = 0;
        let active = null;
        let controller = null;
        const publish = value => onState(value);
        function close() {
            generation += 1;
            controller?.abort(); controller = null; active = null;
            publish({ state: 'closed' });
        }
        async function open(request) {
            controller?.abort();
            const ticket = ++generation;
            controller = new AbortController(); active = request;
            if (!isCurrent(request)) { close(); return; }
            publish({ state: 'loading', request });
            try {
                const result = await read(request, controller.signal);
                if (ticket !== generation) return;
                if (!isCurrent(request)) { close(); return; }
                publish({ state: 'loaded', request, result: validate(result, request) });
            } catch (error) {
                if (ticket !== generation) return;
                if (!isCurrent(request)) { close(); return; }
                publish({ state: 'error', request, error: String(error.message || error) });
            }
        }
        return { open, close, invalidate() { if (active && !isCurrent(active)) close(); } };
    }
    function format(result, labels) {
        const lines = [labels[result.status.state] || result.status.state];
        if (result.canonical_text?.text) lines.push(result.canonical_text.text);
        const identityFields = new Set(['kind', 'id', 'part_id', 'message_id', 'turn_id', 'revision',
            'call_id', 'tool_call_id', 'source_id', 'citation_id', 'origin_tool_call_id']);
        for (const [name, field] of Object.entries(result.fields)) {
            if (identityFields.has(name)) continue;
            const value = field.availability === 'deferred' ? labels.fieldDeferred
                : typeof field.value === 'string' ? field.value : JSON.stringify(field.value, null, 2);
            lines.push(`${labels.fields?.[name] || name}\n${value}`);
        }
        return lines.join('\n\n');
    }
    const api = { descriptor, matches, validate, createLoader, format };
    global.NolaHistoricalPartDetail = api;
    if (typeof module !== 'undefined') module.exports = api;
})(typeof globalThis === 'undefined' ? window : globalThis);
