"""Transparent real-provider relay with boolean evidence for isolated test markers.

Accepts provider_probe.py arguments. NOLA_E2E_MARKERS points to a test-owned
JSON object {label: unique_marker}. No text, headers or credentials are logged.
"""
import hashlib
import importlib.util
import json
import os
import time
from pathlib import Path

source = Path(__file__).resolve().parents[2] / 'scripts/e2e/provider_probe.py'
spec = importlib.util.spec_from_file_location('nola_real_provider_probe', source)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

class MarkerProbe(module.Probe):
    def response_override(self, path, request, headers):
        marker_path = os.environ.get('NOLA_E2E_MARKERS')
        if marker_path and isinstance(request, dict):
            markers = json.loads(Path(marker_path).read_text())
            serialized = json.dumps(request, ensure_ascii=False)
            # Hash is correlation-only; booleans refer exclusively to supplied test markers.
            self.append({'schemaVersion': 1, 'event': 'test.request.markers',
                         'path': path, 'at_ms': time.time_ns() // 1_000_000,
                         'requestValueSha256': hashlib.sha256(serialized.encode()).hexdigest(),
                         'markers': {name: marker in serialized for name, marker in markers.items()}})
        fault_path = os.environ.get('NOLA_E2E_CHILD_FAULT')
        if fault_path and Path(fault_path).exists():
            fault = json.loads(Path(fault_path).read_text())
            messages = request.get('messages', request.get('input', []))
            users = [item for item in messages if isinstance(item, dict) and item.get('role') == 'user'] if isinstance(messages, list) else []
            user_messages = json.dumps(users, ensure_ascii=False)
            if fault.get('enabled') and fault['child_marker'] in user_messages and fault['parent_marker'] not in user_messages:
                self.append({'event': 'test.child_provider_failure', 'at_ms': time.time_ns() // 1_000_000, 'injected': True})
                return 400, json.dumps({'error': {'type': 'invalid_request_error', 'message': 'Isolated child provider failure injection'}}).encode()
        return None

module.Probe = MarkerProbe
if __name__ == '__main__':
    raise SystemExit(module.main())
