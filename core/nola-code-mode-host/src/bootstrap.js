// SPDX-License-Identifier: Apache-2.0
(() => {
    const invoke = globalThis.__nola_invoke;
    const output = globalThis.__nola_output;
    const stringify = JSON.stringify;
    const parse = JSON.parse;
    const catalog = parse(globalThis.__nola_catalog);
    delete globalThis.__nola_catalog;
    const freeze = (value) => {
        if (value !== null && typeof value === 'object') {
            for (const child of Object.values(value)) freeze(child);
            Object.freeze(value);
        }
        return value;
    };
    Object.defineProperty(globalThis, 'ALL_TOOLS', {value: freeze(catalog), writable: false, configurable: false});
    const pending = new Set();
    const allSettled = Promise.allSettled.bind(Promise);
    delete globalThis.__nola_invoke;
    delete globalThis.__nola_output;
    const tools = new Proxy(Object.create(null), {
        get(_target, name) {
            if (typeof name !== 'string' || name === 'then') return undefined;
            return (input = {}) => {
                const task = (async () => {
                    const response = parse(await invoke(name, stringify(input)));
                    if (!response.ok) throw new Error(response.error);
                    return response.value;
                })();
                pending.add(task);
                // Observe both branches without introducing another rejected
                // promise. The original rejection remains visible to callers.
                task.then(() => pending.delete(task), () => pending.delete(task));
                return task;
            };
        },
        set() { return false; },
        defineProperty() { return false; },
        deleteProperty() { return false; },
    });
    Object.defineProperty(globalThis, 'tools', {value: tools, writable: false, configurable: false});
    Object.defineProperty(globalThis, 'text', {value(value) {
        const rendered = typeof value === 'string' ? value : stringify(value);
        if (!output(rendered === undefined ? 'undefined' : rendered)) {
            throw new Error('code_cell_output_rejected');
        }
    }, writable: false, configurable: false});
    // Returned privately to Rust, never installed as a script-accessible global.
    // Even a program that throws or forgets await must settle started callbacks
    // before its parent can complete. Core independently enforces the same rule.
    return async (program) => {
        try {
            await program();
        } finally {
            while (pending.size) await allSettled([...pending]);
        }
    };
})();
