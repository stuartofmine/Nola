import { ModelValidationError } from './modelValidation.ts';

type EditableLocalModel = { id: string; installed: boolean; configurationRevision: number; parameters: object };

// A reload is a new revision baseline, not an in-place update to the old draft.
// Validate before replacing either value so an unavailable model keeps the draft.
export function localModelEditorSnapshot<T extends EditableLocalModel>(models: T[], id: string) {
    const model = models.find(candidate => candidate.id === id && candidate.installed);
    if (!model) throw new ModelValidationError({ code: 'modelValidation.localMissing' });
    if (!Number.isSafeInteger(model.configurationRevision) || model.configurationRevision < 0) {
        throw new ModelValidationError({ code: 'modelValidation.localRevision' });
    }
    return { model, parameters: { ...model.parameters } as T['parameters'] };
}
