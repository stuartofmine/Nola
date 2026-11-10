// SPDX-License-Identifier: Apache-2.0
/** Only the owned local Host can supply a verified package to this session. */
export function isSoftwareUpdateDownload(value: string, hostOrigin: string | undefined): boolean {
    if (!hostOrigin) return false;
    try {
        const url = new URL(value);
        return url.origin === hostOrigin && !url.username && !url.password && !url.hash
            && url.pathname === '/api/software-update/package'
            && [...url.searchParams.keys()].length === 1
            && /^[a-f0-9]{64}$/.test(url.searchParams.get('release_id') ?? '');
    } catch { return false; }
}
