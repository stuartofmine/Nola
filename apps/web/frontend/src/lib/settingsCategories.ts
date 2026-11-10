export const settingsCategories = ['general', 'providers', 'agent', 'connections', 'data', 'diagnostics'] as const;
export type SettingsCategory = typeof settingsCategories[number];

/** Existing notification links now open the notification section in General. */
export function resolveSettingsCategory(value: string | null): SettingsCategory | null {
    if (value === 'notifications') return 'general';
    return settingsCategories.find(category => category === value) ?? null;
}
