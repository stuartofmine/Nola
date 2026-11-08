-- SPDX-License-Identifier: Apache-2.0
-- Input paths must match Core's serialized field names, not Rust field names.
DROP VIEW IF EXISTS core_delivery_witness_projection;
CREATE VIEW core_delivery_witness_projection AS
SELECT conversation_id, revision AS checkpoint_revision, event_cursor,
    json_object(
        'session_id', json(checkpoint_json -> '$.session_id'),
        'core_revision', json(checkpoint_json -> '$.revision'),
        'state', json_object(
            'session_id', json(checkpoint_json -> '$.state.session_id'),
            'execution', json_object(
                'active_turn', CASE WHEN json_type(checkpoint_json, '$.state.execution.active_turn') = 'object'
                    THEN json_object(
                        'turn_id', json(checkpoint_json -> '$.state.execution.active_turn.turn_id'),
                        'status', json(checkpoint_json -> '$.state.execution.active_turn.status'))
                    ELSE NULL END,
                'permissions', json_object('pending', json((
                SELECT json_group_array(json_object(
                    'request_id', json(p.value -> '$.request_id'),
                    'continuation', json(COALESCE(p.value -> '$.continuation', '{"type":"core_tool"}'))
                )) FROM json_each(checkpoint_json, '$.state.execution.permissions.pending') p
            )))),
            'host', json_object('pending_host_effects', json_object('effects', json((
                SELECT json_group_object(e.key, json_object(
                    'effect_id', json(e.value -> '$.effect_id'),
                    'request_id', json(e.value -> '$.request_id'),
                    'permission_request_id', CASE
                        WHEN e.value ->> '$.request.payload.type' = 'request_permission'
                        THEN json(e.value -> '$.request.payload.permission_request_id') ELSE NULL END,
                    'session_id', json(e.value -> '$.session_id'),
                    'turn_id', json(e.value -> '$.turn_id'),
                    'host_id', json(e.value -> '$.host_id'),
                    'lease_id', json(e.value -> '$.lease_id'),
                    'catalog_revision', json(e.value -> '$.catalog_revision'),
                    'availability_epoch', json(e.value -> '$.availability_epoch'),
                    'replay_safety', json(e.value -> '$.replay_safety'),
                    'last_response_sequence', json(e.value -> '$.last_response_sequence')
                )) FROM json_each(checkpoint_json, '$.state.host.pending_host_effects.effects') e
            )))))) AS witness_json
FROM core_checkpoints JOIN core_checkpoint_bodies USING(conversation_id);
