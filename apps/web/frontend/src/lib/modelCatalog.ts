import type { RemoteProfile } from './modelProfiles';

export type CatalogSnapshot = { profiles: RemoteProfile[]; default_chat_model_resolution?: unknown };

export async function loadModelCatalog(request: <R>(path: string) => Promise<R>): Promise<CatalogSnapshot> {
    const { profiles, default_chat_model_resolution } = await request<{ profiles: RemoteProfile[]; default_chat_model_resolution?: unknown }>('/api/model-profiles');
    return { profiles, ...(default_chat_model_resolution === undefined ? {} : {default_chat_model_resolution}) };
}

export function replaceSavedProfile(profiles: RemoteProfile[], saved: RemoteProfile): RemoteProfile[] {
    return profiles.map(profile => profile.id === saved.id ? saved : profile);
}
