import { getContext, setContext } from 'svelte';
import type { UiSnapshot, UiActionRef } from '../../../lib/generativeUi';
const KEY = Symbol('nola.ui.projection');
export interface ProjectionContext {
    snapshot(): UiSnapshot;
    disabled(): boolean;
    actionsDisabled?(): boolean;
    value(instance: string, binding: string, committed: unknown): unknown;
    edit(instance: string, binding: string, value: unknown): void;
    dispatch(action: UiActionRef): void;
}
export function provideProjection(context: ProjectionContext): void { setContext(KEY, context); }
export function useProjection(): ProjectionContext { return getContext<ProjectionContext>(KEY); }

const PRESENTATION_ONLY = Symbol('nola.ui.presentation-only');
export function inheritedPresentationOnly(): () => boolean {
    return getContext<(() => boolean) | undefined>(PRESENTATION_ONLY) ?? (() => false);
}
export function providePresentationOnly(value: () => boolean): void {
    setContext(PRESENTATION_ONLY, value);
}
