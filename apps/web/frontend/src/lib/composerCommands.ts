// UI intents only. No command is admitted as an agent message.
export const composerCommands = [
    { name: 'model', label: 'shell.nav.models', argument: false },
    { name: 'experts', label: 'rooms.experts', argument: false },
    { name: 'resources', label: 'chatInspector.resources', argument: false },
    { name: 'permissions', label: 'chatSession.title', argument: false },
    { name: 'plan', label: 'data_sources.tab.plans', argument: true },
    { name: 'search', label: 'chatPage.find', argument: true },
    { name: 'stop', label: 'chatPage.stop', argument: false },
    { name: 'help', label: 'commands.help', argument: false },
] as const;
export function commandQuery(text: string): string | null {
    return /^\/[^\s/]*$/u.test(text) ? text.slice(1).toLowerCase() : null;
}
export function parseComposerCommand(text: string) {
    if (!text.startsWith('/')) return null;
    const [name, ...rest] = text.slice(1).trim().split(/\s+/u);
    const command = composerCommands.find(item => item.name === name.toLowerCase());
    const argument = rest.join(' ');
    return { command, argument, valid: !!command && (command.argument || !argument) };
}
