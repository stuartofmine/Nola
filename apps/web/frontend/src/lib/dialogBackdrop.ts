/** A dialog itself also receives bubbling events from its contents. Only the
 * native backdrop, outside its border box, is a dismissal surface. */
export function isDialogBackdrop(targetIsDialog: boolean, x: number, y: number,
    bounds: Pick<DOMRect, 'left' | 'right' | 'top' | 'bottom'>): boolean {
    return targetIsDialog && (x < bounds.left || x > bounds.right || y < bounds.top || y > bounds.bottom);
}
