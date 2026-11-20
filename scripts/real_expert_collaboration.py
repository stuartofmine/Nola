# SPDX-License-Identifier: Apache-2.0
"""Real Main Agent → leaf expert acceptance, backed by canonical Store facts.

The model relay never supplies answers. No test creates a Task, Run, receipt or
expert user message; these must be produced by the actual public Main Run path.
"""
import json
import sqlite3
import time
import uuid


def run_collaboration(api, stream, command, profile, roles, database, host_id, client, output, observations, external_executor=None, external_model=None):
    from test_real_expert_roles import answer_text
    from test_real_remote_host_long_tasks import committed_tools

    parent=api('/api/chat/conversations', {'title':'Main Agent expert acceptance'})['conversationId']
    now=lambda:int(time.time()*1000)
    directory=command({'type':'directory'})
    expert_id=str(uuid.uuid4())
    defaults = {'selectedPromptTemplateID': roles['code-reviewer']['id']}
    if external_executor:
        defaults['expertExecutor'] = {'backend': external_executor}
        if external_model:
            defaults['expertExecutor']['model'] = external_model
    else:
        defaults.update(modelSelectionID='configured:'+profile['id'], expertReasoningEffort='high')
    directory=command({'type':'configure_agent','input':{
        'id':expert_id,'expected_directory_revision':directory['revision'],'expected_agent_revision':None,
        'name':'QA code reviewer','description':'Disposable leaf expert','availability':'active',
        'defaults':defaults,'updated_at_ms':now()}})
    expert=next(item for item in directory['agents'] if item['id']==expert_id)
    selected={'agent_id':expert_id,'agent_revision':expert['revision'],'participant_id':None,'context_conversation_id':None}

    def read(sql,args=()):
        with sqlite3.connect(database.as_uri()+'?mode=ro',uri=True) as db:
            db.row_factory=sqlite3.Row
            return [dict(row) for row in db.execute(sql,args)]

    def send(text, *, answer_label_question=False):
        nonlocal selected
        segments = [{'agent_id': expert_id, 'text': '@' + expert['name']}, {'text': ' ' + text}]
        text = ''.join(segment['text'] for segment in segments)
        view=api('/api/chat/conversations/'+parent)
        last=read('SELECT id FROM messages WHERE conversation_id=? ORDER BY ordinal DESC LIMIT 1',(parent,))
        sid=str(uuid.uuid4())
        plan=api('/api/agent/tool-plan/prepare',{'expected_host_id':host_id,'client_id':client,'session_id':parent})['result']
        body={'conversationId':parent,'expected_host_id':host_id,'client_id':client,'submission_id':sid,
            'user_message_id':sid+':user','assistant_message_id':sid+':assistant','execution_backend':'nola',
            'expected_configuration_revision':view['configurationRevision'],
            'expected_parent_message_id':last[0]['id'] if last else None,
            'messages':[{'role':'user','content':text}],'selected_experts':[selected],
            'expert_mentions':{'version':1,'segments':segments},
            'tool_plan_ref':plan['reference'],'max_iterations':32}
        identity={'source_host_id':host_id,'client_id':client,'conversation_id':parent,'submission_id':sid,
            'user_message_id':body['user_message_id'],'assistant_message_id':body['assistant_message_id']}
        prepared=api('/api/chat/agent/v2/submission',{'operation':'prepare','identity':identity,'request':body})['result']
        assert prepared['identity']==identity
        body['request_digest']=prepared['request_digest']
        stream('/api/chat/agent/v2/stream',body)
        deadline=time.monotonic()+240
        asked = []
        while time.monotonic()<deadline:
            rows=read('SELECT status FROM runs WHERE id=?',(sid,))
            if rows and rows[0]['status']=='sealed':break
            if answer_label_question and rows and rows[0]['status']=='waiting_input' and not asked:
                path = '/api/chat/conversations/'+parent+'/user-input-waits'
                record = next((r for r in api(path)['waits'] if r['run_id'] == sid), None)
                if record:
                    wait = record['wait']
                    answers = []
                    for question in wait['question_set']['questions']:
                        assert question['allow_freeform'] and 'label' in question['prompt'].lower()
                        asked.append(question['prompt'])
                        answers.append({'question_id':question['id'], 'value':{'type':'text','text':'ORCHID'}})
                    reply = api(path+'/respond', {'run_id':sid, 'response':{
                        'command_id':str(uuid.uuid4()), 'wait_id':wait['wait_id'], 'expected_revision':wait['revision'],
                        'response_schema_digest':wait['response_schema_digest'], 'response':{'type':'answers','answers':answers}}})
                    assert reply['type'] == 'resolved'
            time.sleep(.2)
        else:raise TimeoutError('Main Run did not canonically seal')
        association=command({'type':'for_conversation','conversation_id':parent})
        member=next(item for item in association['room']['room']['participants'] if item['agent_id']==expert_id and item['active'])
        selected={'agent_id':expert_id,'agent_revision':member['agent_revision'],
            'participant_id':member['id'],'context_conversation_id':member['context_conversation_id']}
        rows = read("SELECT content_text,content_json FROM message_parts WHERE message_id=? AND kind='text'", (sid+':user',))
        assert len(rows) == 1 and rows[0]['content_text'] == text
        frozen = json.loads(rows[0]['content_json'])['expert_mentions']
        assert frozen['segments'] == segments and frozen['selected_experts'] == [selected]
        tasks=read("SELECT id,status,owner_session_id,child_session_id,parent_task_id,metadata_json FROM durable_tasks WHERE kind='agent' AND owner_session_id=? ORDER BY created_at_ms,id",(parent,))
        current=[row for row in tasks if json.loads(row['metadata_json'])['expert_target']['source_run_id']==sid]
        assert len(current)==1 and current[0]['status']=='sealed','Main must own one actual completed expert Task'
        task=current[0];pin=json.loads(task['metadata_json'])['expert_target']
        assert pin['source']=={'kind':'main_agent','context_conversation_id':parent}
        assert task['child_session_id']==member['context_conversation_id'] and task['parent_task_id'] is None
        assert pin['target']['participant_id']==member['id']
        assert len(read('SELECT task_id FROM runtime_task_run_bindings WHERE task_id=? AND run_id=?',(task['id'],task['id'])))==1
        leaf_tasks = read("SELECT kind FROM durable_tasks WHERE owner_session_id=?", (member['context_conversation_id'],))
        assert all(external_executor and row['kind'] == 'external_executor' for row in leaf_tasks), 'Unexpected nested leaf Task'
        assert not any(call['tool'] in {'delegate_expert','expert_exchange','spawn_agent','request_user_input'} for call in committed_tools(database,member['context_conversation_id']))
        child_view=api('/api/chat/conversations/'+member['context_conversation_id'])
        assert child_view['expertContextOwner']['isCurrent'] is True
        assert child_view['expertParentConversationId']==parent
        return "\n".join([*asked, answer_text(database,sid)]),task,answer_text(database,task['id'])

    if external_executor:
        result, task, answer = send('Use delegate_expert to ask the selected reviewer to multiply 29 by 31. '
            'Have it answer EXPERT_PRODUCT=<number> without tools. Wait for the result, then reply MAIN_PRODUCT=<number>.')
        assert 'MAIN_PRODUCT=899' in result and 'EXPERT_PRODUCT=899' in answer, (result, answer)
        metadata = json.loads(task['metadata_json'])
        backend = 'opencode' if external_executor == 'open_code' else external_executor
        role = roles['code-reviewer']
        assert any(item['component_id'] == 'prompt_template:'+role['id'] and role['prompt'] in item['content']
                   for item in metadata['expert_target_prompt_definitions'])
        assert metadata['execution_backend'] == backend
        assert metadata['delegated_invocation']['execution_backend'] == backend
        assert metadata['delegated_invocation']['external_execution_model'] == external_model
        rows = read("SELECT status, metadata_json FROM durable_tasks WHERE owner_session_id=? AND kind='external_executor'",
                    (task['child_session_id'],))
        assert any(row['status'] == 'completed'
                   and json.loads(row['metadata_json']).get('backend_id') == backend
                   and json.loads(row['metadata_json']).get('model_id') == external_model
                   for row in rows), 'External expert must have a completed receipt for the selected executor and model'
        followup, second, remembered = send('Via delegate_expert ask the same reviewer: "What product did you report earlier '
            'in your own conversation? Return EXPERT_RECALL=<number> and do not call tools." Omit numbers and the previous answer '
            'from your brief. Wait for the reviewer, then return MAIN_RECALL=<number>.')
        assert 'EXPERT_RECALL=899' in remembered and 'MAIN_RECALL=899' in followup, (followup, remembered)
        assert task['id'] != second['id'] and task['child_session_id'] == second['child_session_id']
        assert '899' not in json.loads(second['metadata_json'])['expert_exchange_context']['text']
        completed = read("SELECT id, child_session_id FROM durable_tasks WHERE kind='external_executor' AND status='completed' "
                         "AND owner_session_id=?", (task['child_session_id'],))
        assert len(completed) == 2
        return {'persistent_external_expert': {'backend': backend, 'model': external_model, 'parent': parent, 'child': task['child_session_id'],
                'tasks': [task['id'], second['id']], 'native_executor_completions': len(completed), 'own_history_reused': True}}
    first,one,target_answer=send('Use the selected reviewer via delegate_expert. Ask it to compute 17*19 and return TARGET_OWN_HIGH=323. '
        'This is a short text protocol test; do not create artifacts or call file/shell tools. '
        'Wait/read its actual completed result; do not calculate it yourself. Reply MAIN_RESULT=323 only after it returned the answer.')
    assert 'MAIN_RESULT=323' in first and 'TARGET_OWN_HIGH=323' in target_answer
    second,two,target_memory=send('Ask the same selected reviewer, via delegate_expert: "What multiplication result did you give previously in your own conversation? '
        'Use your own history and reply TARGET_MEMORY=<result>. Do not use tools." '
        'Do not repeat operands or answer in the brief. Wait for its actual result, then reply MAIN_MEMORY=<result>.')
    assert 'MAIN_MEMORY=323' in second and 'TARGET_MEMORY=323' in target_memory
    meta=json.loads(two['metadata_json']);brief=meta['expert_exchange_context']['text']
    assert '323' not in brief and '17*19' not in brief and '17 * 19' not in brief
    question,three,needs_label=send('Via delegate_expert ask the selected reviewer: "We need a missing label. '
        'Return NEEDS_LABEL and the question Which label should I use? to your Main Agent. Do not ask the user directly or call tools." '
        'Wait for that actual result, then ask me which label to use. Do not invent a label.', answer_label_question=True)
    assert 'NEEDS_LABEL' in needs_label and 'label' in question.lower()
    fourth,four,clarified=send('The label is ORCHID. Ask the same selected reviewer via delegate_expert to use my provided label, '
        'reply EXPERT_LABEL=ORCHID, and use no tools. Wait/read its real result and finish MAIN_LABEL=ORCHID.')
    assert 'MAIN_LABEL=ORCHID' in fourth and 'EXPERT_LABEL=ORCHID' in clarified
    tasks=[one,two,three,four]
    assert len({row['id'] for row in tasks})==4 and len({row['child_session_id'] for row in tasks})==1
    assert len(read("SELECT id FROM messages WHERE conversation_id=? AND role='user'",(parent,)))==4
    leaf=[row for row in observations if row['review_role']]
    assert leaf and all(not row['delegation_tools'] for row in leaf),'Experts must not dispatch Tasks or directly request user input'
    assert any(row['effort']=='high' for row in leaf)
    evidence={'passed':True,'main_conversation':parent,'shared_tasks':4,'main_user_messages':4,
        'leaf_expert':True,'own_history_reused':True,'clarification_through_main':True,
        'persistent_target_context':one['child_session_id']}
    (output/'main-expert-evidence.json').write_text(json.dumps(evidence,indent=2))
    return {'main_agent_expert_dispatch':evidence}
