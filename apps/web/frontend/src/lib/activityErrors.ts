import { ApiError } from './api.ts';

/** UI state stores stable keys, never remote bodies or already-translated text. */
export function activityErrorKey(error: unknown): string {
    if (error instanceof ApiError && [401, 403].includes(error.status)) return 'activity.errorAccess';
    return 'activity.errorLoad';
}
