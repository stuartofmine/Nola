import type { ChatConversation } from './chatConversations.ts';
import type { RemoteProfile } from './modelProfiles.ts';

export interface LocalTestModel {
    id: string;
    displayName?: string;
    name?: string;
    installed: boolean;
    running: boolean;
    selected: boolean;
}

export function activeApiTestModel(profiles: RemoteProfile[]): RemoteProfile | undefined {
    return profiles.find(profile => profile.selected && profile.enabled && profile.endpoint.enabled !== false
        && profile.capabilities.includes('chat'));
}

export function activeLocalTestModel(models: LocalTestModel[]): LocalTestModel | undefined {
    return models.find(model => model.selected && model.installed);
}

export function modelTestParametersUrl(target: 'local' | 'cloud', modelId: string): string {
    if (!modelId.trim()) throw new Error('modelTest.modelUnavailable');
    const tab = target === 'cloud' ? 'api' : 'local';
    return `/models?tab=${tab}&configure=${encodeURIComponent(modelId)}`;
}

export function modelTestConversationRequest(source: 'local' | 'cloud', modelId: string, title: string) {
    if (!modelId.trim()) throw new Error('modelTest.modelUnavailable');
    return { title, selectedModelSource: source, selectedModelId: modelId, executionBackend: 'nola' };
}

export function modelTestRunRequest(conversation: ChatConversation, clientId: string, submissionId: string, prompt: string, language: string, timezone: string) {
    if (!prompt.trim() || !conversation.conversationId.trim() || !clientId.trim() || !submissionId.trim()) {
        throw new Error('modelTest.invalidRequest');
    }
    return {
        conversationId: conversation.conversationId,
        client_id: clientId,
        submission_id: submissionId,
        user_message_id: `${submissionId}:user`,
        assistant_message_id: `${submissionId}:assistant`,
        messages: [{ role: 'user', content: prompt }],
        current_user_display_text: prompt,
        timezone,
        language,
        execution_backend: 'nola',
        permission_profile: 'cautious',
        agentMode: false,
        model_test_mode: true,
    };
}
