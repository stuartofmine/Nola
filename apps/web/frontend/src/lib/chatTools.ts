export interface ChatToolCatalog {
    names: string[];
}

export interface ChatBrowserConnection {
    connected: boolean;
    hostCount: number;
}

export function parseChatToolCatalog(value: unknown): ChatToolCatalog {
    if (!value || typeof value !== 'object') throw new Error('Invalid tool catalog');
    const tools = (value as { tools?: unknown }).tools;
    if (!Array.isArray(tools)) throw new Error('Invalid tool catalog');
    const names = tools.map(tool => {
        if (!tool || typeof tool !== 'object' || typeof (tool as { name?: unknown }).name !== 'string') {
            throw new Error('Invalid tool catalog entry');
        }
        return (tool as { name: string }).name.trim();
    }).filter(Boolean);
    return { names: [...new Set(names)].sort((a, b) => a.localeCompare(b)) };
}

export function parseChatBrowserConnection(value: unknown): ChatBrowserConnection {
    if (!value || typeof value !== 'object') throw new Error('Invalid browser status');
    const status = value as { connected?: unknown; hosts?: unknown };
    if (typeof status.connected !== 'boolean' || !Array.isArray(status.hosts)) throw new Error('Invalid browser status');
    return { connected: status.connected, hostCount: status.hosts.length };
}
