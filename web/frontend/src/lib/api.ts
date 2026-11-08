import { get } from 'svelte/store';
import { locale } from './i18n.ts';

export class ApiError extends Error {
    status: number;
    code?: string;
    constructor(status: number, message: string, code?: string) {
        super(message); this.status = status; this.code = code;
    }
}

export function csrfToken(cookie: string): string | undefined {
    const item = cookie.split(';').map(value => value.trim()).find(value => value.startsWith('nola_web_csrf='));
    return item?.slice('nola_web_csrf='.length);
}

/** Shared same-origin boundary for both JSON requests and streaming responses. */
export async function apiResponse(path: string, options: RequestInit = {}): Promise<Response> {
    // Credentials and CSRF tokens never leave the same-origin API boundary.
    if (!path.startsWith('/api/') || path.includes('\\')) throw new Error('Invalid API path');
    const headers = new Headers(options.headers);
    if (!headers.has('Accept-Language')) headers.set('Accept-Language', get(locale));
    if (options.body) headers.set('Content-Type', 'application/json');
    const token = csrfToken(document.cookie);
    if (token && options.method && !['GET', 'HEAD'].includes(options.method.toUpperCase())) headers.set('X-Nola-CSRF', token);
    return fetch(path, { ...options, headers, credentials: 'same-origin' });
}

export async function api<T>(path: string, options: RequestInit = {}): Promise<T> {
    const response = await apiResponse(path, options);
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new ApiError(response.status, value.error || value.detail || `HTTP ${response.status}`, value.code);
    return value as T;
}

export function errorMessage(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
}
