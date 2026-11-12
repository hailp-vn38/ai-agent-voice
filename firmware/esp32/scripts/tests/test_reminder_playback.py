import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PUSH = ROOT / "main" / "push"


class ReminderPlaybackTest(unittest.TestCase):
    def run_driver(self, body: str):
        source = (
            '#include <cassert>\n#include "reminder_playback.h"\n'
            "using namespace reminder_playback;\nint main() {\n"
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

    def test_stream_end_alone_is_not_played(self):
        self.run_driver(
            """
            Tracker t;
            assert(t.OnTtsStart("r1", "a1", 0));
            assert(t.OnTtsStop("r1", "a1") == Action::kNone);
            // output still has queued audio
            assert(t.OnPlaybackDrained(false, 10).kind == Action::kNone);
            auto a = t.OnPlaybackDrained(true, 20);
            assert(a.kind == Action::kSendPlayed && a.reminder_id == "r1" && a.attempt_id == "a1");
            // duplicate drain event never re-sends
            assert(t.OnPlaybackDrained(true, 30).kind == Action::kNone);
            """
        )

    def test_failure_and_abort(self):
        self.run_driver(
            """
            Tracker t;
            t.OnTtsStart("r1", "a1", 0);
            auto a = t.OnFailure("playback_aborted", 5);
            assert(a.kind == Action::kSendFailed && a.reason == "playback_aborted");
            assert(a.attempt_id == "a1");
            assert(t.OnFailure("decode_failed", 6).kind == Action::kNone);
            assert(t.OnPlaybackDrained(true, 7).kind == Action::kNone);
            Tracker idle;
            assert(idle.OnFailure("decode_failed", 1).kind == Action::kNone);
            """
        )

    def test_result_must_match_attempt_and_sequential_reminders(self):
        self.run_driver(
            """
            Tracker t;
            t.BeginWakeSession(0);
            t.OnTtsStart("r1", "a1", 100);
            t.OnTtsStop("r1", "a1");
            t.OnPlaybackDrained(true, 200);
            assert(!t.OnDeliveryResult("r1", "other", 210));
            assert(t.OnDeliveryResult("r1", "a1", 220));
            // a late duplicate delivery_result is harmless
            assert(!t.OnDeliveryResult("r1", "a1", 225));
            // second reminder in the same session, before the grace expires
            assert(t.Tick(230) == Action::kNone);
            assert(t.OnTtsStart("r2", "a2", 300));
            t.OnTtsStop("r2", "a2");
            assert(t.OnPlaybackDrained(true, 400).attempt_id == "a2");
            assert(t.OnDeliveryResult("r2", "a2", 410));
            // nothing more: closes after the grace
            assert(t.Tick(410 + kGraceAfterResultMs - 1) == Action::kNone);
            assert(t.Tick(410 + kGraceAfterResultMs) == Action::kCloseSession);
            """
        )

    def test_result_timeout_and_empty_wake_session(self):
        self.run_driver(
            """
            Tracker t;
            t.BeginWakeSession(0);
            assert(t.Tick(kEmptySessionTimeoutMs - 1) == Action::kNone);
            assert(t.Tick(kEmptySessionTimeoutMs) == Action::kCloseSession);

            Tracker u;
            u.BeginWakeSession(0);
            u.OnTtsStart("r1", "a1", 10);
            u.OnTtsStop("r1", "a1");
            u.OnPlaybackDrained(true, 20);
            assert(u.Tick(20 + kResultTimeoutMs - 1) == Action::kNone);
            assert(u.Tick(20 + kResultTimeoutMs) == Action::kCloseSession);

            Tracker normal;  // a normal session never self-closes
            assert(normal.Tick(1000000) == Action::kNone);
            """
        )

    def test_duplicate_start_ignored(self):
        self.run_driver(
            """
            Tracker t;
            assert(t.OnTtsStart("r1", "a1", 0));
            assert(!t.OnTtsStart("r1", "a1", 1));
            assert(!t.OnTtsStart("", "a1", 1));
            """
        )


if __name__ == "__main__":
    unittest.main()
