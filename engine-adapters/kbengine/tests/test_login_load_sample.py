"""End-to-end login load test sample against a real KBEngine cluster
(WM-0217; docs/kbengine.md Target Deployment Shape, Mode A).

Runs examples/login_load/run_e2e.sh: a real cluster (machine .. loginapp)
with a reachable database, a real bots process carrying the Bridge, and
the sample driver playing the Supervisor over a Unix socket. Asserts the
swell completes: every planned bot surfaces as BotCreated and the session
tears down cleanly.

Needs a prepared KBEngine workspace (WAVEMAKER_KBE_ROOT / KBE_ASSETS env,
see examples/login_load/README.md) — without one the test is skipped, so
plain CI gates stay green. The skip guard is the honesty guard: a faked
cluster would prove nothing.
"""

import os
import subprocess
import unittest
from pathlib import Path

SAMPLE_DIR = Path(__file__).resolve().parent.parent / "examples" / "login_load"


@unittest.skipUnless(
    os.environ.get("WAVEMAKER_E2E_CLUSTER"),
    "needs a real KBEngine cluster (set WAVEMAKER_E2E_CLUSTER=1 and "
    "WAVEMAKER_KBE_ROOT, see examples/login_load/README.md)",
)
class TestLoginLoadSample(unittest.TestCase):
    def test_swell_logs_in_all_planned_bots(self):
        result = subprocess.run(
            ["sh", str(SAMPLE_DIR / "run_e2e.sh")],
            capture_output=True,
            text=True,
            timeout=300,
        )
        self.assertEqual(
            result.returncode, 0,
            "sample run failed (rc=%s)\nstdout:\n%s\nstderr:\n%s"
            % (result.returncode, result.stdout, result.stderr),
        )
        self.assertIn("result: PASS", result.stdout)
        # The driver prints the per-type event totals it observed.
        self.assertIn("BotCreated", result.stdout)


if __name__ == "__main__":
    unittest.main()
