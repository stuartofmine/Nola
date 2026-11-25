export const workspaceDraftsContext = Symbol('nola.workspaceDrafts');

export interface WorkspaceDrafts {
    readonly needsUnloadConfirmation: boolean;
    clear(): void;
}

export function createWorkspaceDrafts(): WorkspaceDrafts {
    return {
        get needsUnloadConfirmation(): boolean {
            return false;
        },
        clear(): void {},
    };
}
