// SPDX-License-Identifier: Apache-2.0
import { writable } from 'svelte/store';

export type ToastItem = {
    id: number;
    message: string;
    tone: 'info' | 'success' | 'error';
};

let nextId = 1;
const { subscribe, update } = writable<ToastItem[]>([]);

export const toasts = { subscribe };

export function pushToast(message: string, tone: ToastItem['tone'] = 'info', durationMs = 2600): void {
    const id = nextId++;
    update(items => [...items.slice(-2), { id, message, tone }]);
    if (durationMs > 0) {
        setTimeout(() => dismissToast(id), durationMs);
    }
}

export function dismissToast(id: number): void {
    update(items => items.filter(item => item.id !== id));
}
