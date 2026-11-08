export type PanelSide = 'start' | 'end';
export interface SwipePoint { x: number; y: number; time: number }

export function panelOutwardSign(side: PanelSide, rtl = false): number {
    return (side === 'end' ? 1 : -1) * (rtl ? -1 : 1);
}

export function completesPanelSwipe(start: SwipePoint, end: SwipePoint, width: number, direction: number): boolean {
    const dx = (end.x - start.x) * direction;
    const dy = Math.abs(end.y - start.y);
    const elapsed = end.time - start.time;
    return Number.isFinite(dx + dy + width + elapsed) && width > 0 && elapsed >= 0 && elapsed <= 700
        && dx >= Math.max(48, Math.min(96, width * 0.2)) && dx > dy * 1.5;
}
