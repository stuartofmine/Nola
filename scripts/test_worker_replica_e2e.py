#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Real Worker + local D1 replica acceptance; no production configuration."""
import base64
import hashlib
import hmac
import json
import os
from pathlib import Path
import secrets
import signal
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
WORKER = ROOT / "legacy/backend/worker"
WRANGLER = WORKER / "node_modules/wrangler/bin/wrangler.js"


def token(secret, identity):
    def encode(value):
        return base64.urlsafe_b64encode(json.dumps(value, separators=(",", ":")).encode()).decode().rstrip("=")
    message = encode({"alg": "HS256", "typ": "JWT"}) + "." + encode({
        "sub": identity, "client_uuid": identity, "exp": int(time.time()) + 300,
    })
    signature = base64.urlsafe_b64encode(hmac.new(secret.encode(), message.encode(), hashlib.sha256).digest()).decode().rstrip("=")
    return message + "." + signature


def main():
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
    with tempfile.TemporaryDirectory(prefix="nola-replica-worker-e2e-") as directory:
        root = Path(directory)
        secret = secrets.token_hex(32)
        config = root / "wrangler.json"
        config.write_text(json.dumps({
            "name": "nola-replica-e2e", "main": str(WORKER / "src/index.ts"),
            "compatibility_date": "2026-05-10",
            "vars": {"NOLA_BACKEND_ENV": json.dumps({"SERVER_JWT_SECRET": secret})},
            "d1_databases": [{"binding": "DB", "database_name": "replica-e2e",
                "database_id": "00000000-0000-0000-0000-000000000001",
                "migrations_dir": str(WORKER / "migrations")}],
        }))
        environment = {key: value for key, value in os.environ.items()
                       if not key.startswith(("CLOUDFLARE_", "NOLA_BACKEND_", "WRANGLER_"))}
        environment["WRANGLER_SEND_METRICS"] = "false"
        prefix = ["node", str(WRANGLER)]
        options = ["--config", str(config), "--local", "--persist-to", str(root / "state")]

        def run(*arguments):
            result = subprocess.run(prefix + list(arguments) + options, cwd=root, env=environment,
                                    capture_output=True, text=True, timeout=60)
            if result.returncode:
                raise RuntimeError(result.stdout[-4000:] + result.stderr[-4000:])
            return result.stdout

        run("d1", "migrations", "apply", "replica-e2e")
        base = f"http://127.0.0.1:{port}"
        identity = "11111111111111111111111111111111"
        bearer = token(secret, identity)

        def request(path, body=None, auth=bearer):
            data = json.dumps(body).encode() if body is not None else None
            req = urllib.request.Request(base + path, data=data, headers={
                "Authorization": "Bearer " + auth, "Content-Type": "application/json",
            })
            try:
                response = urllib.request.urlopen(req, timeout=15)
            except urllib.error.HTTPError as error:
                response = error
            with response:
                raw = response.read(12 * 1024 * 1024 + 1)
                assert len(raw) <= 12 * 1024 * 1024
                return response.status, json.loads(raw)

        with (root / "worker.log").open("w+") as log:
            process = subprocess.Popen(prefix + ["dev", "--ip", "127.0.0.1", "--port", str(port)] + options,
                cwd=root, env=environment, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                deadline = time.monotonic() + 30
                while True:
                    if process.poll() is not None:
                        raise RuntimeError("isolated Worker exited during startup")
                    try:
                        with urllib.request.urlopen(base + "/healthz", timeout=0.5) as response:
                            if response.status == 200:
                                break
                    except (OSError, urllib.error.URLError):
                        pass
                    if time.monotonic() >= deadline:
                        raise TimeoutError("isolated Worker startup exceeded 30s")
                    time.sleep(0.1)

                if "--with-cli" in sys.argv:
                    cli_environment = dict(environment,
                        NOLA_REPLICA_E2E_URL=base,
                        NOLA_REPLICA_E2E_TOKEN=token(secret, "33333333333333333333333333333333"))
                    cli = subprocess.run([
                        "cargo", "test", "-p", "nola_cli", "--lib",
                        "replica_sync::tests::isolated_worker_round_trip_uses_real_cli_transport",
                        "--", "--ignored", "--exact", "--nocapture",
                    ], cwd=ROOT, env=cli_environment, capture_output=True, text=True, timeout=180)
                    if cli.returncode:
                        raise RuntimeError("CLI/Worker integration failed:\n" + cli.stdout[-6000:] + cli.stderr[-6000:])
                    print(json.dumps({"cli_worker_core_round_trip": True, "records": 7,
                        "multiple_push_batches": True, "original_values_equal": True, "no_echo": True,
                        "conversation_messages": 7, "conversation_facts": 9,
                        "unicode_range_reads": True, "bounded_projection": True, "profile_restored": True}))

                if "--with-cli" in sys.argv:
                    chunk_environment = dict(environment,
                        NOLA_REPLICA_E2E_URL=base,
                        NOLA_REPLICA_E2E_TOKEN=token(secret, "44444444444444444444444444444444"))
                    chunked = subprocess.run([
                        "cargo", "test", "-p", "nola_cli", "--lib",
                        "replica_sync::tests::isolated_worker_chunked_body_survives_http_and_database_restarts",
                        "--", "--ignored", "--exact", "--nocapture",
                    ], cwd=ROOT, env=chunk_environment, capture_output=True, text=True, timeout=180)
                    if chunked.returncode:
                        raise RuntimeError("Chunked CLI/Worker integration failed:\n" + chunked.stdout[-6000:] + chunked.stderr[-6000:])
                    evidence = next(line for line in chunked.stdout.splitlines() if line.startswith("chunked_worker_acceptance "))
                    print(json.dumps({"chunked_cli_worker_round_trip": True, "evidence": evidence,
                        "published_store_fixture": True, "mac_import_entry_point": False,
                        "restarted_both_databases": True, "ack_before_reference": True}))

                endpoint = "/v1/sync/replicas"
                fixtures = []
                for index, kind in enumerate(["conversation", "conversation_append", "conversation_metadata"] + ["record"] * 7):
                    change = {"operationId": f"op-{index}", "entityType": kind, "entityId": f"entity-{index}",
                        "revision": 1, "deleted": False, "originDeviceId": "device-e2e",
                        "payload": {"text": "x" * 700_000 if index >= 3 else "incremental evidence"}}
                    status, pushed = request(endpoint, {"changes": [change]})
                    assert status == 200, (status, pushed)
                    assert pushed["results"][0]["status"] == "accepted", pushed
                    fixtures.append(change)
                status, replay = request(endpoint, {"changes": [fixtures[0]]})
                assert status == 200 and replay["results"][0]["status"] == "idempotent", replay
                conflict = dict(fixtures[0], payload={"text": "different"})
                status, rejected = request(endpoint, {"changes": [conflict]})
                assert status == 200 and rejected["results"][0]["status"] == "operation_conflict", rejected

                cursor, pages, received = 0, 0, []
                while True:
                    status, page = request(endpoint + f"?cursor={cursor}&limit=100")
                    assert status == 200, (status, page)
                    assert page["changes"], page
                    assert page["nextCursor"] > cursor
                    cursor = page["nextCursor"]
                    received.extend(page["changes"])
                    pages += 1
                    assert pages <= 10, "pagination made insufficient progress"
                    if not page["hasMore"]:
                        break
                assert pages >= 2, "byte budget did not split oversized aggregate"
                assert [item["operationId"] for item in received] == [item["operationId"] for item in fixtures]
                assert [item["payload"] for item in received] == [item["payload"] for item in fixtures]
                status, isolated = request(endpoint, auth=token(secret, "22222222222222222222222222222222"))
                assert status == 200 and isolated["changes"] == [] and isolated["nextCursor"] == 0
                assert request(endpoint + "?limit=NaN")[0] == 400
                assert request(endpoint, {"changes": [], "padding": "x" * (4 * 1024 * 1024)})[0] == 413
                status, tail = request(endpoint + f"?cursor={cursor}")
                assert status == 200 and tail["changes"] == [] and tail["nextCursor"] == cursor

                run("d1", "execute", "replica-e2e", "--command",
                    "INSERT INTO canonicalreplicachangerecord(client_uuid,operation_id,entity_type,entity_id,revision,payload_json,origin_device_id) "
                    f"VALUES('{identity}','corrupt','record','corrupt',1,'{{','test');")
                for _ in range(2):
                    status, error = request(endpoint + f"?cursor={cursor}")
                    assert status == 500, (status, error)
                    assert "cursor unchanged" in json.dumps(error), error
                print(json.dumps({"ok": True, "real_worker_d1": True, "records": len(received),
                    "pages": pages, "replay": True, "account_isolation": True,
                    "oversize_rejected": True, "corrupt_payload_rejected": True}))
            except BaseException:
                log.flush()
                log.seek(0)
                print(log.read()[-6000:])
                raise
            finally:
                try:
                    os.killpg(process.pid, signal.SIGTERM)
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait(timeout=5)
                except ProcessLookupError:
                    pass


if __name__ == "__main__":
    main()
