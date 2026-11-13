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
            auto signal = [&](int64_t expires_at) { return Fields{1, "wake", "r1", now - 5, expires_at}; };
            Fields ok = signal(now + 1000);
            assert(Evaluate(ok, now, false) == Verdict::kAccept);
            Fields wrong_version = ok; wrong_version.v = 2;
            assert(Evaluate(wrong_version, now, false) == Verdict::kWrongVersion);
            Fields no_version = ok; no_version.v.reset();
            assert(Evaluate(no_version, now, false) == Verdict::kWrongVersion);
            Fields wrong_type = ok; wrong_type.type = "ping";
            assert(Evaluate(wrong_type, now, false) == Verdict::kWrongType);
            Fields no_type = ok; no_type.type.reset();
            assert(Evaluate(no_type, now, false) == Verdict::kWrongType);
            // A signal that cannot be told apart from others is never acted on.
            Fields no_id = ok; no_id.id.reset();
            assert(Evaluate(no_id, now, false) == Verdict::kUnidentified);
            Fields empty_id = ok; empty_id.id = "";
            assert(Evaluate(empty_id, now, false) == Verdict::kUnidentified);
            Fields no_at = ok; no_at.at_ms.reset();
            assert(Evaluate(no_at, now, false) == Verdict::kUnidentified);
            assert(Evaluate(signal(now), now, false) == Verdict::kExpired);
            assert(Evaluate(signal(now - 1), now, false) == Verdict::kExpired);
            Fields no_expiry = ok; no_expiry.expires_at_ms.reset();
            assert(Evaluate(no_expiry, now, false) == Verdict::kExpired);
            assert(Evaluate(ok, now, true) == Verdict::kBusy);
            // Unset device clock: the signal is ignored, not treated as fresh.
            assert(Evaluate(ok, 1000, false) == Verdict::kClockUnset);
            assert(Evaluate(ok, 0, false) == Verdict::kClockUnset);
            """
        )

    def test_retry_signal_differs_from_the_retained_copy_of_a_handled_one(self):
        self.run_driver(
            """
            using namespace wake_signal;
            Identity handled{"r1", 2000};
            assert(Identity({"r1", 2000}) == handled);   // retained copy: same signal
            assert(!(Identity{"r1", 7000} == handled));  // retry checkpoint: new signal
            assert(!(Identity{"r2", 2000} == handled));  // another Reminder
            """
        )

    def test_pending_wake_retries_with_backoff_until_settled_or_expired(self):
        self.run_driver(
            """
            using namespace wake_signal;
            PendingWake p;
            assert(!p.ShouldOpen(0));
            p.Accept(Accepted{{"r1", 9000}, 200000}, 1000);
            assert(p.pending() && (p.identity() == Identity{"r1", 9000}));
            assert(p.ShouldOpen(1000));
            // open failed: wait, then try again with a doubled wait
            p.OnSessionFailed(1000);
            assert(!p.ShouldOpen(1000 + PendingWake::kRetryInitialMs - 1));
            assert(p.ShouldOpen(1000 + PendingWake::kRetryInitialMs));
            p.OnSessionFailed(6000);
            assert(!p.ShouldOpen(6000 + 2 * PendingWake::kRetryInitialMs - 1));
            assert(p.ShouldOpen(6000 + 2 * PendingWake::kRetryInitialMs));
            // the retained copy arriving again must not reset the backoff
            p.OnSessionFailed(16000);
            p.Accept(Accepted{{"r1", 9000}, 200000}, 16500);
            assert(!p.ShouldOpen(16500));
            // the same Reminder at its retry checkpoint is a different signal: starts fresh
            p.Accept(Accepted{{"r1", 9500}, 200000}, 16500);
            assert(p.identity().at_ms == 9500 && p.ShouldOpen(16500));
            p.Settle();
            assert(!p.pending() && !p.ShouldOpen(17000));
            // the backoff is capped
            PendingWake capped;
            capped.Accept(Accepted{{"r3", 1}, 10000000}, 0);
            int64_t now = 0;
            for (int i = 0; i < 12; i++) capped.OnSessionFailed(now);
            assert(capped.ShouldOpen(now + PendingWake::kRetryMaxMs));
            assert(!capped.ShouldOpen(now + PendingWake::kRetryMaxMs - 1));
            // an expired signal is given up
            PendingWake expired;
            expired.Accept(Accepted{{"r4", 1}, 5000}, 0);
            expired.OnSessionFailed(0);
            assert(!expired.ShouldOpen(5000));
            assert(!expired.pending());
            """
        )


if __name__ == "__main__":
    unittest.main()
