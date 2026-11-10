/** Return the horizontal scroll position needed to reveal a tab within its rail. */
export function visibleTabScrollLeft(scrollLeft: number, railLeft: number, railRight: number, tabLeft: number, tabRight: number): number {
    if (tabLeft < railLeft) return Math.max(0, scrollLeft - (railLeft - tabLeft));
    if (tabRight > railRight) return scrollLeft + (tabRight - railRight);
    return scrollLeft;
}
