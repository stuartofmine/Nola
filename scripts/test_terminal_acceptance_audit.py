# SPDX-License-Identifier: Apache-2.0
"""Current terminal gate rejects self-reported/erased/mixed-owner evidence."""
import copy
import json
from pathlib import Path

import pytest
import terminal_acceptance_audit as audit_module


def fixture(tmp_path, monkeypatch):
    root = Path(tmp_path)
    frames, actions, runs = [], [], []
    for index, kind in enumerate(("answered", "cancelled", "answered"), 1):
        runs.append({"runId": f"run{index}", "conversationId": "owner", "status": "sealed", "kind": kind,
                     "sealDigest": str(index) * 64, "resultDigest": str(index) * 64})

    def frame(text):
        frames.append({"frameId": len(frames) + 1, "rawOffset": len(frames) + 1, "rows": 24, "columns": 100,
                       "screen": text + "\nType a message\nContext 1%", "cursorRow": 22, "cursorColumn": 2})

    def action(name, **facts):
        actions.append({"action": name, "frameId": len(frames),
                        "monotonicNs": len(actions) * 10_000_000, **facts})

    frame("ready")
    action("long_input")
    for first in range(1, 41, 4):
        frame("\n".join(f"SCROLL-{line:02} word" for line in range(first, first + 4)))
    action("formal_seal", runs=copy.deepcopy(runs[:1]))
    action("page_up")
    frame("SCROLL-01 word")
    action("page_up_witness")
    action("page_down")
    frame("SCROLL-40 word")
    action("page_down_witness")
    action("cancel_input")
    frame("CANCEL-BEGIN-741\nEsc interrupt")
    action("escape")
    frame("cancelled")
    action("formal_seal", runs=copy.deepcopy(runs[:2]))
    action("followup_input", priorRunId="run2")
    frame("FOLLOWUP-OK-741")
    action("formal_seal", runs=copy.deepcopy(runs))
    paths = [root / name for name in ("raw", "frames", "actions", "metrics", "audit", "database")]
    paths[0].write_bytes(b"capture")
    metrics = {"schemaVersion": 1, "passed": True, "sourceConfigurationModified": False,
               "secretCleanupSucceeded": True, "retainedEvidenceContainsSecret": False}
    live = {"runs": runs, "formalServiceRead": True}

    def save():
        paths[1].write_text("".join(json.dumps(row) + "\n" for row in frames))
        paths[2].write_text("".join(json.dumps(row) + "\n" for row in actions))
        paths[3].write_text(json.dumps(metrics))
        paths[4].write_text(json.dumps(live))
        paths[5].write_bytes(b"owned database test double")

    save()
    monkeypatch.setattr(audit_module, "run_oracle", lambda binary, requests:
        [{"frames": copy.deepcopy(frames)}, {"runs": copy.deepcopy(runs), "formalServiceRead": False}])
    return paths, frames, actions, runs, save


def test_current_terminal_gate_replays_frames_and_binds_three_formal_outcomes(tmp_path, monkeypatch):
    paths, *_ = fixture(tmp_path, monkeypatch)
    assert audit_module.verify(*paths, Path("trusted-oracle"))["pageUpDownVerified"] is True


@pytest.mark.parametrize("fault", ["empty_frames", "deleted_database", "wrong_seal", "duplicate_footer",
                                 "cancel_not_sealed_before_followup", "fake_ansi_words"])
def test_terminal_gate_rejects_missing_or_fabricated_ui_evidence(tmp_path, monkeypatch, fault):
    paths, frames, actions, runs, save = fixture(tmp_path, monkeypatch)
    if fault == "empty_frames":
        frames.clear()
    elif fault == "wrong_seal":
        runs[1]["kind"] = "answered"
    elif fault == "duplicate_footer":
        frames[2]["screen"] += "\nContext 99%"
    elif fault == "cancel_not_sealed_before_followup":
        seals = [index for index, row in enumerate(actions) if row["action"] == "formal_seal"]
        actions[seals[1]], actions[seals[1] + 1] = actions[seals[1] + 1], actions[seals[1]]
    save()
    if fault == "deleted_database":
        paths[-1].unlink()
    if fault == "fake_ansi_words":
        # Raw success words erased inside a frame are not visible testimony.
        paths[0].write_bytes(b"SCROLL-01 SCROLL-40 CANCEL-BEGIN-741 FOLLOWUP-OK-741\r\x1b[2Kgoned")
        monkeypatch.setattr(audit_module, "run_oracle", lambda binary, requests:
            [{"frames": [{**row, "screen": "gone"} for row in frames]}, {"runs": runs}])
    with pytest.raises(ValueError):
        audit_module.verify(*paths, Path("trusted-oracle"))
