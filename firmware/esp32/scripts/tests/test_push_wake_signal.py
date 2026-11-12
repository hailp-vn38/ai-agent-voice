import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PUSH = ROOT / "main" / "push"


class WakeSignalDecisionTest(unittest.TestCase):
    def run_driver(self, body: str):
        source = (
            '#include <cassert>\n#include "wake_signal.h"\nint main() {\n'
            + textwrap.dedent(body)
            + "\n}\n"
        )
        with tempfile.TemporaryDirectory() as tmp:
            src = Path(tmp) / "t.cc"
            exe = Path(tmp) / "t"
            src.write_text(source)
            subprocess.run(
                ["c++", "-std=c++20", "-I", str(PUSH), str(src), "-o", str(exe)],
                check=True,
            )
            subprocess.run([str(exe)], check=True)

    def test_decisions(self):
        self.run_driver(
            """
            using namespace wake_signal;
            const int64_t now = 1800000000000LL;
            Fields ok{1, "wake", now + 1000};
            assert(Evaluate(ok, now, false) == Verdict::kAccept);
            assert(Evaluate(Fields{2, "wake", now + 1000}, now, false) == Verdict::kWrongVersion);
            assert(Evaluate(Fields{std::nullopt, "wake", now + 1000}, now, false) == Verdict::kWrongVersion);
            assert(Evaluate(Fields{1, "ping", now + 1000}, now, false) == Verdict::kWrongType);
            assert(Evaluate(Fields{1, std::nullopt, now + 1000}, now, false) == Verdict::kWrongType);
            assert(Evaluate(Fields{1, "wake", now}, now, false) == Verdict::kExpired);
            assert(Evaluate(Fields{1, "wake", now - 1}, now, false) == Verdict::kExpired);
            assert(Evaluate(Fields{1, "wake", std::nullopt}, now, false) == Verdict::kExpired);
            assert(Evaluate(ok, now, true) == Verdict::kBusy);
            // Unset device clock: the signal is ignored, not treated as fresh.
            assert(Evaluate(Fields{1, "wake", now + 1000}, 1000, false) == Verdict::kClockUnset);
            assert(Evaluate(Fields{1, "wake", now + 1000}, 0, false) == Verdict::kClockUnset);
            """
        )

    def test_retained_replay_after_resubscribe_is_ignored(self):
        self.run_driver(
            """
            using namespace wake_signal;
            assert(IsRetainedReplay("r1", 2000, "r1", 2000, 100));
            assert(!IsRetainedReplay("r1", 2000, "r1", 2000, 6000));
            assert(!IsRetainedReplay("r2", 2000, "r1", 2000, 100));
            assert(!IsRetainedReplay("r1", 3000, "r1", 2000, 100));
            assert(!IsRetainedReplay("r1", 2000, "", 0, 100));
            """
        )


if __name__ == "__main__":
    unittest.main()
