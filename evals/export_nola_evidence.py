#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Read-only evidence export from an isolated benchmark database, not a verifier."""
import json
import gzip
import hashlib
from pathlib import Path
import re
import sqlite3
import sys
from urllib.parse import urlparse, unquote


def credential_values(config: Path | None) -> list[bytes]:
    """Read only explicit secret fields; never include configuration in evidence."""
    if config is None:
        return []
    values = []

    def visit(value):
        if isinstance(value, dict):
            for key, item in value.items():
                if key.lower() in {'api_key', 'access_token', 'refresh_token', 'password', 'secret'}:
                    if isinstance(item, str) and item:
                        values.append(item.encode())
                else:
                    visit(item)
        elif isinstance(value, list):
            for item in value:
                visit(item)

    configuration = json.loads(config.read_text(encoding='utf-8'))
    if not isinstance(configuration, dict):
        raise ValueError('credential configuration must be an object')
    visit(configuration)
    return sorted(set(values), key=len, reverse=True)


def materialize_tool_outputs(event_log: Path, output_root: Path, destination: Path,
                             secrets: list[bytes] | None = ()) -> dict:
    """Archive referenced output bytes, not display previews or arbitrary task files.

    Original and archived hashes differ when redaction occurs. References remain
    associated with the exact event/call; a missing blob is never called complete.
    """
    manifest = {'event_log_status': 'missing', 'references': [],
                'redaction_policy': 'configured_secret_values_and_bearer_tokens',
                'all_references_materialized': False}
    if not event_log.exists():
        return manifest
    manifest['event_log_status'] = 'read'
    manifest['invalid_json_lines'] = 0
    opener = gzip.open if event_log.suffix == '.gz' else open
    with opener(event_log, 'rt', encoding='utf-8', errors='replace') as stream:
        for line_number, line in enumerate(stream, 1):
            try:
                event = json.loads(line)
            except ValueError:
                manifest['invalid_json_lines'] += 1
                continue
            if not isinstance(event, dict):
                continue
            trace = event.get('trace') or {}
            if not isinstance(trace, dict):
                continue
            payload = trace.get('payload') or {}
            if not isinstance(payload, dict):
                continue
            result = payload.get('result') or {}
            if not isinstance(result, dict):
                continue
            identity = {'event_id': event.get('event_id', trace.get('event_id')),
                      'run_id': event.get('run_id', trace.get('run_id')),
                      'sequence': event.get('sequence', trace.get('sequence')),
                      'trace_sequence': trace.get('sequence'), 'call_id': payload.get('call_id'),
                      'tool_name': payload.get('tool_name'), 'event_log_line': line_number}
            metadata = payload.get('metadata') or {}
            if (trace.get('kind') == 'tool.progress'
                    and payload.get('summary') == 'tool_media.prepared'
                    and isinstance(metadata, dict)):
                manifest.setdefault('media_request_preparations', []).append({
                    **identity, 'request_id': metadata.get('request_id'),
                    'effect_id': metadata.get('effect_id'), 'artifacts': metadata.get('artifacts', []),
                    'model_receipt_proven': False})
            if trace.get('kind') != 'tool.completed':
                continue
            for artifact in result.get('artifacts', []):
                if not isinstance(artifact, dict):
                    continue
                mime = artifact.get('mime_type') or ''
                uri = artifact.get('uri') or ''
                if not (mime.startswith('image/') or mime == 'application/pdf'):
                    continue
                parsed = urlparse(uri)
                record = {**identity, 'source_reference': uri, 'mime_type': mime,
                          'media_binding_verified': False, 'model_receipt_proven': False}
                manifest.setdefault('media_references', []).append(record)
                if parsed.scheme != 'file' or parsed.netloc not in {'', 'localhost'}:
                    record['status'] = 'unsupported_media_uri'
                    continue
                source = Path(unquote(parsed.path))
                archive_blob(record, source, output_root, destination, secrets)
                if record.get('status') == 'archived':
                    record['media_binding_verified'] = source.name == f"media-{record['source_sha256']}.bin"
                    if not record['media_binding_verified']:
                        record['status'] = 'archived_unbound_media'
            reference = result.get('full_output_ref')
            if not isinstance(reference, str) or not reference:
                continue
            record = {**identity, 'source_reference': reference}
            manifest['references'].append(record)
            archive_blob(record, Path(reference), output_root, destination, secrets)
    manifest['all_references_materialized'] = all(
        item['status'] == 'archived' for item in manifest['references'])
    manifest['all_media_references_materialized'] = all(
        item['status'] == 'archived' for item in manifest.get('media_references', []))
    return manifest


def archive_blob(record: dict, source: Path, output_root: Path, destination: Path,
                 secrets: list[bytes] | None) -> None:
    if secrets is None:
        record['status'] = 'not_archived_credentials_unavailable'
        return
    root = output_root.resolve()
    try:
        if not source.is_absolute() or not source.resolve().is_relative_to(root) or source.is_symlink():
            record['status'] = 'rejected_outside_output_store'
            return
        # Reject links anywhere inside the output store, even an internal alias.
        lexical_root = output_root.absolute()
        if not source.is_relative_to(lexical_root) or any(
            item.is_symlink() for item in (source, *source.parents)
            if item == lexical_root or item.is_relative_to(lexical_root)
        ):
            record['status'] = 'rejected_symlink_path'
            return
        data = source.read_bytes()
        record['source_sha256'] = hashlib.sha256(data).hexdigest()
        record['source_bytes'] = len(data)
        archived = data
        for secret in secrets:
            archived = archived.replace(secret, b'[REDACTED]')
        archived = re.sub(rb'(?i)(Bearer\s+)[A-Za-z0-9._~+/=-]+', rb'\1[REDACTED]', archived)
        digest = hashlib.sha256(archived).hexdigest()
        destination.mkdir(parents=True, exist_ok=True)
        blob = destination / (digest + '.bin')
        blob.write_bytes(archived)
        record.update(status='archived', archived_path=destination.name + '/' + blob.name,
                      archived_sha256=digest, archived_bytes=len(archived), redacted=archived != data)
    except FileNotFoundError:
        record['status'] = 'missing'
    except OSError as error:
        record.update(status='unreadable', error_kind=type(error).__name__)


def archive_with_credentials(event_log: Path, output_root: Path, destination: Path,
                             config: Path | None) -> dict:
    try:
        secrets = credential_values(config)
    except (OSError, ValueError, UnicodeError) as error:
        # Keep the call/reference inventory, but never copy unredacted bodies
        # when the isolated credential source cannot be read safely.
        manifest = materialize_tool_outputs(event_log, output_root, destination, None)
        manifest['credential_status'] = 'unavailable'
        manifest['credential_error_kind'] = type(error).__name__
        manifest['all_references_materialized'] = False
        return manifest
    manifest = materialize_tool_outputs(event_log, output_root, destination, secrets)
    manifest['credential_status'] = 'loaded' if config is not None else 'not_supplied'
    return manifest


def collect(database: Path) -> dict:
    # Never create a missing database or export configuration/checkpoint tables.
    connection = sqlite3.connect(database.resolve().as_uri() + '?mode=ro', uri=True)
    connection.row_factory = sqlite3.Row
    try:
        connection.execute('BEGIN')
        queries = {
            'runs': 'SELECT id, conversation_id, status, created_at_ms, completed_at_ms FROM runs ORDER BY id',
            'model_attempts': "SELECT id,run_id,operation_id,state,json_extract(record_json,'$.model') AS model,json_extract(record_json,'$.usage') AS usage_json FROM run_attempts ORDER BY id",
            'runtime_tasks': 'SELECT id,kind,status,owner_session_id,parent_task_id,child_session_id,agent_type,created_at_ms,started_at_ms,ended_at_ms FROM durable_tasks ORDER BY id',
            'compactions': "SELECT run_id,sequence,event_kind,created_at_ms,payload_json FROM run_events WHERE event_kind IN ('compaction_requested','compaction_completed','compaction_failed') ORDER BY run_id,sequence",
            'messages': 'SELECT id, conversation_id, run_id, role, status, ordinal FROM messages ORDER BY conversation_id, ordinal',
            'parts': 'SELECT id, message_id, kind, ordinal, state, content_text FROM message_parts ORDER BY message_id, ordinal',
            'body_references': 'SELECT part_id, field, reference_json FROM message_part_body_references ORDER BY part_id, field',
            'seals': 'SELECT run_id, conversation_id, seal_json, seal_digest FROM assistant_run_terminal_seals ORDER BY run_id',
            'stream_heads': 'SELECT part_id, head_json FROM canonical_stream_heads ORDER BY part_id',
            'stream_manifests': 'SELECT part_id, root, previous_root, operation, head_json FROM canonical_stream_manifests ORDER BY part_id, root',
            'stream_chunks': 'SELECT part_id, generation, chunk_index, offset_bytes, text, manifest_root FROM canonical_stream_chunks ORDER BY part_id, generation, chunk_index',
            'journal_heads': 'SELECT owner, sequence, digest, retained_floor FROM assistant_journal_heads ORDER BY owner',
            'journal_records': 'SELECT owner, sequence, operation_id, source_revision, header_json, hex(batch) AS batch_hex FROM assistant_journal_records ORDER BY owner, sequence',
            'journal_parts': 'SELECT owner, part_id, head_json FROM assistant_journal_parts ORDER BY owner, part_id',
            'journal_manifests': 'SELECT root, owner, part_id, field, generation, byte_count, chunk_count, previous_root, chunk_digest FROM assistant_journal_manifests ORDER BY owner, part_id, field, generation, byte_count',
            'journal_chunks': 'SELECT digest, hex(bytes) AS bytes_hex FROM assistant_journal_chunks ORDER BY digest',
            'journal_manifest_chunks': 'SELECT owner, part_id, field, generation, chunk_index, digest, byte_count, result_root FROM assistant_journal_manifest_chunks ORDER BY owner, part_id, field, generation, chunk_index',
        }
        return {
            'schema_version': 1,
            'scope': 'isolated_evaluation_database',
            'seal_digests_verified': False,
            'referenced_bodies_materialized': False,
            # Preserve generations and manifest identities; concatenating all
            # chunks would fabricate an answer after a stream replacement.
            'stream_bodies_materialized': False,
            **{name: [dict(row) for row in connection.execute(sql)]
               for name, sql in queries.items()},
        }
    finally:
        connection.close()


if __name__ == '__main__':
    try:
        evidence = collect(Path(sys.argv[1]))
        evidence['database_export_status'] = 'collected'
    except sqlite3.Error as error:
        # A crashed/missing DB must not discard independently recorded tool
        # outputs. Do not expose SQL/configuration through exception messages.
        evidence = {'schema_version': 2, 'scope': 'isolated_evaluation_database',
                    'database_export_status': 'failed', 'database_error_kind': type(error).__name__,
                    'seal_digests_verified': False, 'referenced_bodies_materialized': False,
                    'stream_bodies_materialized': False}
    destination = Path(sys.argv[2])
    if len(sys.argv) > 3:
        evidence['tool_output_archive'] = archive_with_credentials(
            Path(sys.argv[3]), Path(str(Path(sys.argv[1])) + '.outputs'),
            destination.parent / 'tool-outputs',
            Path(sys.argv[4]) if len(sys.argv) > 4 else None)
    temporary = destination.with_suffix('.tmp')
    temporary.write_text(json.dumps(evidence, ensure_ascii=False), encoding='utf-8')
    temporary.replace(destination)
