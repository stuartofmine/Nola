import type { RemoteProfile } from './modelProfiles';

export type CatalogSnapshot = { profiles: RemoteProfile[] };

export async function loadModelCatalog(request: <R>(path: string) => Promise<R>): Promise<CatalogSnapshot> {
    const { profiles } = await request<{ profiles: RemoteProfile[] }>('/api/model-profiles');
    return { profiles };
}

export function replaceSavedProfile(profiles: RemoteProfile[], saved: RemoteProfile): RemoteProfile[] {
    return profiles.map(profile => profile.id === saved.id ? saved : profile);
}
