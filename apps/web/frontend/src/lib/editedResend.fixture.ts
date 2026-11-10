import { createEditedResendDraft } from './editedResend.ts';
import type { EditedResendEnvelope } from './editedResend.ts';
import { createChatTurn } from './chatTurn.ts';
import { terminalFixture } from './assistantTerminal.fixture.ts';

export function editedFixture() {
    const draft=createEditedResendDraft({hostId:'host',clientId:'client',conversationId:'source',conversationRevision:4,messageId:'source-user',messageRevision:7,originalText:'Original'},'Edited',
        {operationId:'11111111-1111-4111-8111-111111111111',targetId:'22222222-2222-4222-8222-222222222222',createdAtMs:100});
    const p=draft.input;
    const envelope:EditedResendEnvelope={host_id:'host',client_id:'client',prepared:{input:{...p},source:{conversation_id:'source',message_id:'source-user',run_id:'old-run',assistant_message_id:'old-assistant',
        body_sha256:'a'.repeat(64),invocation_config_sha256:'b'.repeat(64),model:{version:1,model_capability:{profile_id:"fixture",revision:1,direct_tool_capability_ids:[]},primary:{kind:'http'},fallbacks:[],generation:{},output_admission:{}}},
        run_id:p.operation_id,user_message_id:`${p.operation_id}:user`,assistant_message_id:`${p.operation_id}:assistant`,admitted_run_id:null},
        target:{id:p.target_conversation_id,title:p.title,revision:1,configuration_revision:1}};
    const turn=createChatTurn({hostId:'host',clientId:'client',conversationId:p.target_conversation_id,generation:1},
        {submissionId:p.operation_id,userMessageId:envelope.prepared.user_message_id,assistantMessageId:envelope.prepared.assistant_message_id});
    const receipt={protocol_version:1 as const,identity:{source_host_id:'host',client_id:'client',conversation_id:p.target_conversation_id,
        submission_id:p.operation_id,user_message_id:envelope.prepared.user_message_id,assistant_message_id:envelope.prepared.assistant_message_id},
        request_digest:'c'.repeat(64),phase:'admitted' as const,admission:terminalFixture(turn).seal.admission,rejection_code:null};
    return {draft,envelope,receipt};
}
