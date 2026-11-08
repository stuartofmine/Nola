import importlib.util
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location(
    "measure_mac_release_memory", Path(__file__).parents[1] / "measure_mac_release_memory.py"
)
memory = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(memory)


class ProcessTreeMemoryTests(unittest.TestCase):
    def test_rss_snapshot_includes_only_root_and_transitive_children(self):
        snapshot = subprocess.CompletedProcess([], 0, stdout=(
            "30 20 3 /tmp/worker\n"
            "10 1 10 /tmp/Nola App/Nola\n"
            "99 1 900 /tmp/unrelated\n"
            "20 10 2 /tmp/service\n"
        ))
        with patch.object(memory.subprocess, "run", return_value=snapshot), \
                patch.object(memory, "physical_footprint") as footprint:
            rows = memory.process_rows(10, include_footprint=False)
        self.assertEqual([row["pid"] for row in rows], [10, 20, 30])
        self.assertEqual(sum(row["rss_bytes"] for row in rows), 15 * 1024)
        self.assertTrue(all("phys_footprint_bytes" not in row for row in rows))
        footprint.assert_not_called()

    def test_detailed_inspection_failure_cannot_silently_pass_a_budget(self):
        snapshot = subprocess.CompletedProcess([], 0, stdout="10 1 10 /tmp/Nola\n")
        with patch.object(memory.subprocess, "run", return_value=snapshot), \
                patch.object(memory, "physical_footprint", side_effect=RuntimeError("exited")):
            with self.assertRaisesRegex(RuntimeError, "exited"):
                memory.process_rows(10)


if __name__ == "__main__":
    unittest.main()
