# SPDX-License-Identifier: Apache-2.0
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("provider_stream_acceptance",
    Path(__file__).resolve().parents[1] / "provider_stream_acceptance.py")
acceptance = importlib.util.module_from_spec(spec)
spec.loader.exec_module(acceptance)


class ProviderStreamAcceptanceTests(unittest.TestCase):
    def record(self, **changes):
        return dict(event="provider.response", stream=True, status=200,
                    finishReasons=["stop"], responseDoneMarkerCount=1,
                    responseOversizedLineCount=0,
                    responseTextDeltaCount=2, responseTextBytes=10, **changes)

    def test_text_stream_passes(self):
        self.assertEqual(acceptance.require_text_stream([self.record()]), 1)

    def test_tool_only_and_reasoning_are_not_text(self):
        record = self.record(responseToolNames=["requirements_update"],
                             responseReasoningDeltaCount=10)
        record.update(responseTextDeltaCount=0, responseTextBytes=0)
        with self.assertRaises(ValueError):
            acceptance.require_text_stream([record])

    def test_complete_responses_stream_passes_without_chat_done(self):
        record = self.record()
        record.update(finishReasons=["completed"], responseDoneMarkerCount=0)
        self.assertEqual(acceptance.require_text_stream([record]), 1)

    def test_partial_terminal_or_lossy_trace_does_not_pass(self):
        for change in ({"finishReasons": []}, {"finishReasons": ["length"]},
                       {"finishReasons": ["stop", "length"]},
                       {"responseDoneMarkerCount": 0},
                       {"responseOversizedLineCount": 1}):
            with self.subTest(change=change):
                record = self.record()
                record.update(change)
                with self.assertRaises(ValueError):
                    acceptance.require_text_stream([record])

    def test_nonstream_failed_interrupted_or_empty_do_not_pass(self):
        for change in ({"stream": False}, {"status": 500}, {"error": "failed"},
                       {"injectedInterruption": True}, {"responseTextBytes": 0}):
            with self.subTest(change=change):
                record = self.record()
                record.update(change)
                with self.assertRaises(ValueError):
                    acceptance.require_text_stream([record])
        with self.assertRaises(ValueError):
            acceptance.require_text_stream([])
