export type ProviderSummary = {
    endpoints: number;
    models: number;
    selected: { id: string; name: string } | null;
};

/** Project only the fields displayed here; never retain endpoint URLs or credentials. */
export function providerSummary(value: unknown): ProviderSummary {
    const record = (item: unknown): Record<string, unknown> => {
        if (!item || typeof item !== 'object' || Array.isArray(item)) throw new Error('Invalid model catalog');
        return item as Record<string, unknown>;
    };
    const string = (item: unknown): string => {
        if (typeof item !== 'string' || !item.trim()) throw new Error('Invalid model catalog');
        return item;
    };
    const profiles = record(value).profiles;
    if (!Array.isArray(profiles)) throw new Error('Invalid model catalog');
    const ids = new Set<string>(), endpoints = new Set<string>();
    let selected: ProviderSummary['selected'] = null;
    for (const item of profiles) {
        const profile = record(item);
        const id = string(profile.id), name = string(profile.displayName);
        if (ids.has(id) || typeof profile.selected !== 'boolean') throw new Error('Invalid model catalog');
        ids.add(id);
        endpoints.add(string(record(profile.endpoint).id));
        if (profile.selected) {
            if (selected) throw new Error('Ambiguous selected model');
            selected = { id, name };
        }
    }
    return { endpoints: endpoints.size, models: ids.size, selected };
}
