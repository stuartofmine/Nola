// Nola projection adapter; upstream visual component interfaces retained.
export interface Binding<T = unknown> {
    readonly value: T;
    set(next: T): void;
    readonly path: string | null;
}
export interface ValidationResult { valid: boolean; errors: string[] }
export interface SlotNode { id: string; key: string }
export type SlotContent =
    | { kind: 'nodes'; nodes: SlotNode[] }
    | { kind: 'tabs'; tabs: { title: string; nodes: SlotNode[] }[] };
