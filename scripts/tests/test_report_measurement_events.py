# SPDX-License-Identifier: Apache-2.0
import importlib.util
import pathlib
import unittest

spec = importlib.util.spec_from_file_location("report_measurement_events",
    pathlib.Path(__file__).resolve().parents[1] / "report_measurement_events.py")
reporter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reporter)
BUILD = "a" * 64


def event(number, kind, domain="process-a", **changes):
    return {"schema_version": 1, "build_digest": BUILD, "process_id": 1,
        "clock_domain": domain, "observation_id": number, "span_id": 7,
        "monotonic_ns": str(number * 1000000), "elapsed_ns": "1000000" if kind == "end" else None,
        "stage": "host_tool", "event": kind,
        "outcome": "succeeded" if kind == "end" else None, "dropped_total": 0, **changes}


class MeasurementReportTests(unittest.TestCase):
    def joined_fixture(self):
        ids = {name: char * 64 for name, char in (("command","a"),("run","b"),("session","c"),
            ("effect","d"),("request","e"),("attempt","f"))}
        command_context = {"command": ids["command"], "session": ids["session"], "command_kind": "begin_turn"}
        sampling_context = {name: ids[name] for name in ("session","effect","request","attempt")}
        rows = [event(1,"begin",stage="command_execution",span_id=10,context=command_context),
            event(2,"point",stage="command_receipt_committed",span_id=11,outcome="succeeded",
                context={**command_context,"run":ids["run"],"parent_span":10}),
            event(3,"begin",stage="provider_sampling",context=sampling_context),
            event(4,"point",stage="adapter_first_text",span_id=90,context={**sampling_context,"parent_span":7}),
            event(5,"end",stage="provider_sampling",context=sampling_context)]
        manifest = {"samples":[{"command":ids["command"],"result":"succeeded"}]}
        canonical = {"receipts":[{"command":ids["command"],"run":ids["run"],"session":ids["session"],
            "status":"committed","run_binding":"opened_run"}], "attempts":[{**sampling_context,
            "run":ids["run"],"purpose":"conversation","run_kind":"conversation","predecessor":None}]}
        return rows, manifest, canonical

    def test_command_run_attempt_join_uses_all_identities_and_same_clock(self):
        rows, manifest, canonical = self.joined_fixture()
        value = reporter.summarize(rows, BUILD, manifest, canonical)["command_output"]
        self.assertEqual(value["sampling_joins"], {"matched_sampling_spans":1})
        onset = value["stages"]["adapter_first_text"]
        self.assertEqual(onset["all_samples"], 1)
        self.assertEqual(onset["observed_onset_by_sample_result"]["succeeded"]["median_ms"], 3)
        for row in rows[2:]:
            row["clock_domain"] = "other-process-clock"
        value = reporter.summarize(rows, BUILD, manifest, canonical)["command_output"]["stages"]["adapter_first_text"]
        self.assertEqual(value["cross_clock_output_no_latency"], 1)
        self.assertEqual(value["observed_onset_by_sample_result"], {})

    def test_same_session_foreign_run_compaction_and_followup_do_not_steal_first_output(self):
        for variant in ("foreign_run", "compaction", "maintenance", "followup", "foreign_request"):
            rows, manifest, canonical = self.joined_fixture()
            if variant == "foreign_run": canonical["attempts"][0]["run"] = "1"*64
            if variant == "compaction": canonical["attempts"][0]["purpose"] = "compaction"
            if variant == "maintenance": canonical["attempts"][0]["run_kind"] = "maintenance"
            if variant == "followup": canonical["receipts"][0]["run_binding"] = "existing_run"
            if variant == "foreign_request": canonical["attempts"][0]["request"] = "2"*64
            value = reporter.summarize(rows, BUILD, manifest, canonical)["command_output"]["stages"]["adapter_first_text"]
            self.assertEqual(value["all_samples"], 1)
            self.assertEqual(value["observed_onset_by_sample_result"], {})

    def test_retries_keep_canonical_denominator_even_without_sampling(self):
        rows, manifest, canonical = self.joined_fixture()
        canonical["attempts"].append({**canonical["attempts"][0], "attempt":"1"*64, "predecessor":"f"*64})
        value = reporter.summarize(rows, BUILD, manifest, canonical)["command_output"]
        self.assertEqual(value["canonical_attempts"], 2)
        self.assertEqual(value["retry_attempts"], 1)

    def test_missing_predecessor_does_not_make_retry_output_a_proven_run_first_token(self):
        rows, manifest, canonical = self.joined_fixture()
        predecessor = {**canonical["attempts"][0], "attempt":"1"*64, "predecessor":None}
        canonical["attempts"][0]["predecessor"] = predecessor["attempt"]
        canonical["attempts"].append(predecessor)
        value = reporter.summarize(rows, BUILD, manifest, canonical)["command_output"]
        onset = value["stages"]["adapter_first_text"]
        self.assertEqual(onset["samples_with_unobserved_canonical_attempts"], 1)
        self.assertEqual(onset["samples_without_output_proof_for_every_attempt"], 1)
        self.assertEqual(onset["canonical_conversation_attempts"], 2)
        self.assertEqual(onset["sampling_observed_attempts"], 1)
        self.assertEqual(onset["output_observed_attempts"], 1)
        self.assertIn("lossy proxy", value["endpoint"])
        self.assertNotIn("elapsed_by_sample_result", onset)
        self.assertEqual(onset["with_same_clock_observed_onset"], 1)

    def test_command_admission_and_output_order_must_be_valid(self):
        for origin_time, admission_time, category in (("3000000","2000000","invalid_command_admission_order"),
                ("1000000","4500000","output_precedes_admission")):
            rows, manifest, canonical = self.joined_fixture()
            rows[0]["monotonic_ns"], rows[1]["monotonic_ns"] = origin_time, admission_time
            onset = reporter.summarize(rows, BUILD, manifest, canonical)["command_output"]["stages"]["adapter_first_text"]
            self.assertEqual(onset[category], 1)
            self.assertEqual(onset["observed_onset_by_sample_result"], {})

    def receipt(self, number, command="b" * 64, **changes):
        return event(number, "point", stage="command_receipt_committed",
            context={"command": command, "command_kind": "begin_turn"}, outcome="succeeded", **changes)

    def test_manifest_keeps_pre_sampling_failures_and_missing_receipts(self):
        samples = [{"command": c * 64, "result": r} for c, r in
            (("b", "succeeded"), ("c", "failed"), ("d", "cancelled"), ("e", "timeout"))]
        canonical = {"receipts": [{"command": c * 64, "status": s} for c, s in
            (("b", "committed"), ("c", "rejected"), ("d", "committed"), ("e", "absent"))]}
        value = reporter.summarize([self.receipt(1)], BUILD, {"samples": samples}, canonical)["command_receipts"]
        self.assertEqual(value["all_samples"], 4)
        self.assertEqual(value["committed_command_samples"], 2)
        self.assertEqual(value["reconciliation"], {"matched": 1, "no_observed_receipt": 3})
        self.assertEqual(value["sample_results"]["failed"], 1)

    def test_rejected_receipt_is_not_business_accepted_and_replay_is_not_receipt(self):
        rows = [{**self.receipt(1), "outcome": "rejected"},
            event(2, "begin", stage="command_execution", context={"command":"b"*64,"command_kind":"begin_turn"}),
            event(3, "end", stage="command_execution", context={"command":"b"*64,"command_kind":"begin_turn"})]
        value = reporter.summarize(rows, BUILD, {"samples": [{"command": "b"*64, "result": "failed"}]},
            {"receipts": [{"command": "b"*64, "status": "conflict"}]})["command_receipts"]
        self.assertEqual(value["committed_command_samples"], 0)
        self.assertEqual(value["observed_unique_receipts"], 1)
        self.assertEqual(value["reconciliation"], {"matched": 1})

    def test_duplicate_or_conflicting_receipt_points_are_not_selected_by_order(self):
        for rows in ([self.receipt(1), self.receipt(2)],
                [self.receipt(1), {**self.receipt(1), "outcome": "rejected"}]):
            for ordered in (rows, list(reversed(rows))):
                value = reporter.summarize(ordered, BUILD)["command_receipts"]
                self.assertEqual(value["ambiguous_commands"], 1)
                self.assertEqual(value["observed_unique_receipts"], 0)

    def test_canonical_identity_mismatch_and_invalid_manifest_are_not_silent(self):
        experiment = {"samples": [{"command": "b"*64, "result": "failed"}]}
        value = reporter.summarize([self.receipt(1)], BUILD, experiment,
            {"receipts": [{"command": "b"*64, "status": "committed", "run": "c"*64}]})["command_receipts"]
        self.assertEqual(value["reconciliation"], {"receipt_mismatch": 1})
        with self.assertRaises(ValueError):
            reporter.summarize([], BUILD, {"samples": experiment["samples"] * 2}, {"receipts": []})

    def sampling(self, number, kind, **changes):
        return event(number, kind, stage="provider_sampling", context={"attempt": "hashed-a"}, **changes)

    def first_text(self, number, **changes):
        return event(number, "point", span_id=90 + number, stage="adapter_first_text",
            context={"attempt": "hashed-a", "parent_span": 7}, **changes)

    def test_adapter_onset_is_same_clock_and_has_observed_denominator(self):
        rows = [self.sampling(1, "begin"), self.first_text(3), self.sampling(5, "end")]
        report = reporter.summarize(rows, BUILD)["adapter_first_output"]["stages"]
        text = report["adapter_first_text"]
        self.assertEqual(text["observed_sampling_spans"], 1)
        self.assertEqual(text["elapsed_by_sampling_outcome"]["succeeded"]["median_ms"], 2)
        self.assertEqual(report["adapter_first_reasoning"]["without_observed_output"], 1)

    def test_adapter_failure_and_missing_terminal_keep_separate_distributions(self):
        for terminal, outcome in (([], "censored"),
                ([self.sampling(5, "end", outcome="failed")], "failed")):
            report = reporter.summarize([self.sampling(1, "begin"), self.first_text(3)] + terminal,
                BUILD)["adapter_first_output"]["stages"]["adapter_first_text"]
            self.assertEqual(set(report["elapsed_by_sampling_outcome"]), {outcome})

    def test_adapter_ambiguous_foreign_or_out_of_span_point_is_not_latency(self):
        for points in ([self.first_text(2), self.first_text(3)],
                [self.first_text(6)], [self.first_text(0)],
                [{**self.first_text(3), "context": {"attempt": "foreign", "parent_span": 7}}]):
            report = reporter.summarize([self.sampling(1, "begin"), *points, self.sampling(5, "end")],
                BUILD)["adapter_first_output"]["stages"]["adapter_first_text"]
            self.assertEqual(report["invalid_output_observations"], 1)
            self.assertEqual(report["elapsed_by_sampling_outcome"], {})

    def test_adapter_cross_clock_parent_does_not_produce_latency(self):
        report = reporter.summarize([self.sampling(1, "begin"), self.first_text(3, domain="other"),
            self.sampling(5, "end")], BUILD)["adapter_first_output"]
        self.assertEqual(report["rejected_points"], {"missing_sampling_parent": 1})
        self.assertEqual(report["stages"]["adapter_first_text"]["without_observed_output"], 1)

    def test_pair_and_duplicate_source_do_not_double_count(self):
        rows = [event(1, "begin"), event(2, "end")]
        report = reporter.summarize(rows + rows, BUILD)
        stage = report["stages"]["host_tool"]
        self.assertEqual(stage["outcomes"], {"succeeded": 1})
        self.assertEqual(stage["elapsed_by_outcome"]["succeeded"]["p95_ms"], 1)
        self.assertEqual(report["rejected_records"]["duplicate_observation"], 2)

    def test_clock_domains_never_pair(self):
        report = reporter.summarize([event(1, "begin"), event(2, "end", "process-b")], BUILD)
        self.assertEqual(report["stages"]["host_tool"]["unpaired"], 2)
        self.assertEqual(report["stages"]["host_tool"]["elapsed_by_outcome"], {})

    def test_conflicting_identity_never_selects_first_file_as_truth(self):
        rows = [event(1, "begin"), event(2, "end"),
                event(2, "end", outcome="failed", elapsed_ns="9000000")]
        for ordered in (rows, list(reversed(rows))):
            report = reporter.summarize(ordered, BUILD)
            self.assertEqual(report["rejected_records"]["conflicting_observation"], 1)
            self.assertEqual(report["stages"]["host_tool"]["invalid_pairs"], 1)
            self.assertEqual(report["stages"]["host_tool"]["elapsed_by_outcome"], {})

    def test_conflicting_point_invalidates_sampling_latency_in_both_orders(self):
        point = self.first_text(3)
        rows = [self.sampling(1, "begin"), point,
                {**point, "monotonic_ns": "4000000"}, self.sampling(5, "end")]
        for ordered in (rows, list(reversed(rows))):
            report = reporter.summarize(ordered, BUILD)
            onset = report["adapter_first_output"]["stages"]["adapter_first_text"]
            self.assertEqual(onset["invalid_sampling_spans"], 1)
            self.assertEqual(onset["elapsed_by_sampling_outcome"], {})
            self.assertEqual(report["point_counts"], {})
            self.assertEqual(report["stages"]["provider_sampling"]["elapsed_by_outcome"], {})

    def test_conflicting_identity_moved_between_spans_invalidates_both(self):
        rows = [event(1, "begin"), event(2, "end"),
                event(3, "begin", span_id=8), event(2, "end", span_id=8)]
        for ordered in (rows, list(reversed(rows))):
            report = reporter.summarize(ordered, BUILD)
            self.assertEqual(report["stages"]["host_tool"]["invalid_pairs"], 2)
            self.assertEqual(report["stages"]["host_tool"]["elapsed_by_outcome"], {})

    def test_mixed_stage_span_diagnostics_are_independent_of_file_order(self):
        rows = [event(1, "begin"), event(2, "end", stage="host_compaction")]
        forward = reporter.summarize(rows, BUILD)
        reverse = reporter.summarize(list(reversed(rows)), BUILD)
        self.assertEqual(forward, reverse)
        for stage in ("host_tool", "host_compaction"):
            self.assertEqual(forward["stages"][stage]["invalid_pairs"], 1)
            self.assertEqual(forward["stages"][stage]["elapsed_by_outcome"], {})

    def test_failure_and_incomplete_are_not_success_latency(self):
        for outcome in ("failed", "cancelled", "rejected", "incomplete"):
            report = reporter.summarize([event(1, "begin"), event(2, "end", outcome=outcome)], BUILD)
            self.assertEqual(set(report["stages"]["host_tool"]["elapsed_by_outcome"]), {outcome})

    def test_invalid_and_missing_build_are_explicit(self):
        rows = [None, event(1, "begin", build_digest=None), event(2, "end", elapsed_ns="NaN"),
                event(3, "point", stage=[]), event(4, "point", process_id=True)]
        report = reporter.summarize(rows, BUILD)
        self.assertEqual(report["rejected_records"], {"invalid_schema": 4, "different_or_missing_build": 1})

    def test_drops_are_max_per_clock_not_sum_of_snapshots(self):
        report = reporter.summarize([event(1, "point", dropped_total=4),
            event(2, "point", dropped_total=5), event(3, "point", "process-b", dropped_total=2)], BUILD)
        self.assertEqual(report["observed_dropped_lower_bound"], 7)
        self.assertEqual(report["point_counts"], {"host_tool": 3})

    def test_ambiguous_or_backwards_pair_is_not_latency(self):
        for rows in ([event(1, "begin"), event(2, "begin"), event(3, "end")],
                     [event(2, "begin"), event(1, "end")],
                     [event(1, "begin", context={"request": "a"}),
                      event(2, "end", context={"request": "b"})]):
            report = reporter.summarize(rows, BUILD)
            self.assertEqual(report["stages"]["host_tool"]["invalid_pairs"], 1)
            self.assertEqual(report["stages"]["host_tool"]["elapsed_by_outcome"], {})


if __name__ == "__main__":
    unittest.main()
