#ifndef REMINDER_PLAYBACK_H
#define REMINDER_PLAYBACK_H

#include <cstdint>
#include <string>

// Pure state of one Reminder playback attempt (server protocol: `tts` start/stop with
// source=reminder, then `reminder` played/failed -> delivery_result). Host-testable; Application
// owns the I/O. Nothing here persists: no schedule cache, no NVS, no RTC.
namespace reminder_playback {

constexpr int64_t kEmptySessionTimeoutMs = 10000;  // wake session where the server plays nothing
constexpr int64_t kResultTimeoutMs = 8000;         // waiting for delivery_result
constexpr int64_t kGraceAfterResultMs = 3000;      // wait for a following Reminder
constexpr int64_t kPlayingTimeoutMs = 180000;      // safety bound while audio streams

enum class Action { kNone, kSendPlayed, kSendFailed, kCloseSession };

struct Decision {
    Action kind = Action::kNone;
    std::string reminder_id;
    std::string attempt_id;
    std::string reason;
};

class Tracker {
public:
    // Session opened by a Wake Signal: it closes itself when idle (see Tick).
    void BeginWakeSession(int64_t now_ms) {
        wake_session_ = true;
        phase_ = Phase::kNone;
        deadline_ms_ = now_ms + kEmptySessionTimeoutMs;
    }

    void Reset() {
        wake_session_ = false;
        phase_ = Phase::kNone;
        reminder_id_.clear();
        attempt_id_.clear();
        deadline_ms_ = 0;
    }

    // The user started talking inside a wake session: it is theirs now and never self-closes.
    void ReleaseToUser() { wake_session_ = false; }

    bool active() const { return phase_ == Phase::kPlaying || phase_ == Phase::kDraining; }
    bool wake_session() const { return wake_session_; }
    const std::string& attempt_id() const { return attempt_id_; }

    bool OnTtsStart(const std::string& reminder_id, const std::string& attempt_id, int64_t now_ms) {
        if (reminder_id.empty() || attempt_id.empty() || phase_ != Phase::kNone)
            return false;
        reminder_id_ = reminder_id;
        attempt_id_ = attempt_id;
        phase_ = Phase::kPlaying;
        deadline_ms_ = now_ms + kPlayingTimeoutMs;
        return true;
    }

    // End of the network stream: audio may still be queued, so this is never "played".
    Action OnTtsStop(const std::string& reminder_id, const std::string& attempt_id) {
        if (phase_ == Phase::kPlaying && reminder_id == reminder_id_ && attempt_id == attempt_id_) {
            phase_ = Phase::kDraining;
        }
        return Action::kNone;
    }

    Decision OnPlaybackDrained(bool sink_idle, int64_t now_ms) {
        if (phase_ != Phase::kDraining || !sink_idle)
            return {};
        return Await(Action::kSendPlayed, "", now_ms);
    }

    Decision OnFailure(const std::string& reason, int64_t now_ms) {
        if (!active())
            return {};
        return Await(Action::kSendFailed, reason, now_ms);
    }

    bool OnDeliveryResult(const std::string& reminder_id, const std::string& attempt_id,
                          int64_t now_ms) {
        if (phase_ != Phase::kAwaitingResult || reminder_id != reminder_id_ ||
            attempt_id != attempt_id_) {
            return false;
        }
        phase_ = Phase::kNone;
        deadline_ms_ = now_ms + kGraceAfterResultMs;
        return true;
    }

    Action Tick(int64_t now_ms) {
        if (!wake_session_ || now_ms < deadline_ms_)
            return Action::kNone;
        return Action::kCloseSession;
    }

private:
    enum class Phase { kNone, kPlaying, kDraining, kAwaitingResult };

    Decision Await(Action kind, const std::string& reason, int64_t now_ms) {
        phase_ = Phase::kAwaitingResult;
        deadline_ms_ = now_ms + kResultTimeoutMs;
        return {kind, reminder_id_, attempt_id_, reason};
    }

    bool wake_session_ = false;
    Phase phase_ = Phase::kNone;
    std::string reminder_id_;
    std::string attempt_id_;
    int64_t deadline_ms_ = 0;
};

}  // namespace reminder_playback

#endif  // REMINDER_PLAYBACK_H
