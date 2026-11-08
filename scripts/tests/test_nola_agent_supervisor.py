# SPDX-License-Identifier: Apache-2.0
import json
import os
import subprocess
import sys
import unittest
from pathlib import Path

SUPERVISOR = Path(__file__).resolve().parents[2] / 'evals/nola_agent_supervisor.py'


class SupervisorTests(unittest.TestCase):
    def test_child_receives_actual_trial_budget_overriding_inherited_values(self):
        environment = dict(os.environ, NOLA_EXECUTION_DEADLINE_UNIX_MS='1',
                           NOLA_EXECUTION_TIME_LIMIT_SECONDS='28800')
        result = subprocess.run(
            [sys.executable, str(SUPERVISOR), '--seconds', '5', '--', sys.executable, '-c',
             'import os,time,json; print(json.dumps(dict(deadline=int(os.environ["NOLA_EXECUTION_DEADLINE_UNIX_MS"]),'
             'now=time.time_ns()//1000000,seconds=os.environ["NOLA_EXECUTION_TIME_LIMIT_SECONDS"])))'],
            env=environment, capture_output=True, text=True, timeout=8)
        self.assertEqual(result.returncode, 0, result.stderr)
        observed = json.loads(result.stdout)
        self.assertEqual(float(observed['seconds']), 5)
        self.assertGreater(observed['deadline'], observed['now'])
        self.assertLessEqual(observed['deadline'] - observed['now'], 5000)

    def test_deadline_closes_descendant_pipes_even_when_leader_exits_first(self):
        result = subprocess.run(
            [sys.executable, str(SUPERVISOR), '--seconds', '0.2', '--',
             'sh', '-c', 'sleep 30 & echo child-started; wait'],
            capture_output=True, text=True, timeout=8,
        )
        self.assertEqual(result.returncode, 124)
        self.assertIn('child-started', result.stdout)
        timing = json.loads(result.stderr.splitlines()[-1])
        self.assertEqual(timing['type'], 'evaluation_process_timing')
        self.assertEqual(timing['supervisor_exit_code'], 124)
        self.assertGreater(timing['monotonic_elapsed_ms'], 0)
        self.assertAlmostEqual(
            timing['clock_divergence_ms'],
            timing['wall_elapsed_ms'] - timing['monotonic_elapsed_ms'],
            places=5,
        )

    def test_normal_exit_cleans_orphans_and_preserves_exit_status(self):
        result = subprocess.run(
            [sys.executable, str(SUPERVISOR), '--seconds', '5', '--',
             'sh', '-c', 'sleep 30 & echo child-started; exit 7'],
            capture_output=True, text=True, timeout=8,
        )
        self.assertEqual(result.returncode, 7)
        self.assertIn('child-started', result.stdout)
        timing = json.loads(result.stderr.splitlines()[-1])
        self.assertEqual(timing['supervisor_exit_code'], 7)
        self.assertNotIn('command', timing)


if __name__ == '__main__':
    unittest.main()
