// SPDX-License-Identifier: Apache-2.0
export interface WorkArea { x: number; y: number; width: number; height: number }
export interface SavedWindowState { x?: number; y?: number; width: number; height: number; maximized: boolean }

/** Restore the entire usable window, including on smaller or removed monitors. */
export function restoreWindowGeometry(saved: Partial<SavedWindowState>, areas: WorkArea[], primary: WorkArea): SavedWindowState {
    const positive = (value: unknown, fallback: number) => typeof value === 'number' && Number.isFinite(value) && value > 0 ? value : fallback;
    const width = Math.min(3840, Math.max(720, positive(saved.width, 1440)));
    const height = Math.min(2160, Math.max(520, positive(saved.height, 900)));
    const positioned = typeof saved.x === 'number' && Number.isFinite(saved.x) && typeof saved.y === 'number' && Number.isFinite(saved.y);
    let area = primary, overlap = 0;
    if (positioned) for (const candidate of areas) {
        const w = Math.min(saved.x! + width, candidate.x + candidate.width) - Math.max(saved.x!, candidate.x);
        const h = Math.min(saved.y! + height, candidate.y + candidate.height) - Math.max(saved.y!, candidate.y);
        if (w >= 120 && h >= 80 && w * h > overlap) { area = candidate; overlap = w * h; }
    }
    const fitWidth = Math.min(width, area.width), fitHeight = Math.min(height, area.height);
    return {
        width: fitWidth, height: fitHeight, maximized: saved.maximized === true,
        x: positioned && overlap > 0 ? Math.max(area.x, Math.min(saved.x!, area.x + area.width - fitWidth)) : undefined,
        y: positioned && overlap > 0 ? Math.max(area.y, Math.min(saved.y!, area.y + area.height - fitHeight)) : undefined,
    };
}
