import { modelValidationMessages } from './locales/modelValidation.ts';

export type ModelValidationCode = keyof typeof modelValidationMessages.en;
export type ModelValidationIssue = { code: ModelValidationCode; params?: Record<string, string | number>; cause?: ModelValidationIssue };

export class ModelValidationError extends Error {
    readonly issue: ModelValidationIssue;
    constructor(issue: ModelValidationIssue) {
        super(issue.code);
        this.name = 'ModelValidationError';
        this.issue = issue;
    }
}

/** Translate at render time, keeping stored validation independent of the UI locale. */
export function modelValidationText(language: string, issue: ModelValidationIssue | string): string {
    if (typeof issue === 'string') return issue;
    const messages = modelValidationMessages[language as keyof typeof modelValidationMessages] ?? modelValidationMessages.en;
    const params: Record<string, string | number> = { ...issue.params, ...(issue.cause ? { reason: modelValidationText(language, issue.cause) } : {}) };
    return messages[issue.code].replace(/\{([A-Za-z][A-Za-z0-9_]*)\}/g, (placeholder, key: string) =>
        Object.hasOwn(params, key) ? String(params[key]) : placeholder);
}
