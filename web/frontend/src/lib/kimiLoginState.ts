/** Unknown/lost receipts require observation, never a new source mutation. */
export function kimiCanStartLogin(state: string): boolean {
    return ['authenticated', 'unauthenticated', 'expired', 'revoked', 'denied', 'cancelled'].includes(state);
}
