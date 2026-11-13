#ifndef PUSH_WAKE_SIGNAL_H
#define PUSH_WAKE_SIGNAL_H

#include <cstdint>
#include <optional>
#include <string>

// Decision rules for a Device Push Channel Wake Signal. Pure logic so it can be
// tested on the host; JSON extraction lives in device_push_channel.cc.
namespace wake_signal {

constexpr int kSupportedVersion = 1;
constexpr const char* kWakeType = "wake";

struct Fields {
    std::optional<int64_t> v;
    std::optional<std::string> type;
    std::optional<std::string> id;
    std::optional<int64_t> at_ms;
    std::optional<int64_t> expires_at_ms;
};

// Device clock before the OTA response sets it (1970-ish) is "unknown": expiry cannot be judged.
constexpr int64_t kMinPlausibleMs = 1700000000000LL;

enum class Verdict {
    kAccept,
    kWrongVersion,
    kWrongType,
    kUnidentified,
    kClockUnset,
    kExpired,
    kBusy
};

inline Verdict Evaluate(const Fields& f, int64_t now_ms, bool session_active) {
    if (!f.v || *f.v != kSupportedVersion) return Verdict::kWrongVersion;
    if (!f.type || *f.type != kWakeType) return Verdict::kWrongType;
    if (!f.id || f.id->empty() || !f.at_ms) return Verdict::kUnidentified;
    if (now_ms < kMinPlausibleMs) return Verdict::kClockUnset;
    if (!f.expires_at_ms || *f.expires_at_ms <= now_ms) return Verdict::kExpired;
    if (session_active) return Verdict::kBusy;
    return Verdict::kAccept;
}

// What makes one Wake Signal distinct: the Reminder and the due moment or retry checkpoint it
// stands for. The broker re-delivers the retained signal on every subscribe; a copy with the
// identity of a signal already handled is that replay, while a retry carries a new `at`.
struct Identity {
    std::string id;
    int64_t at_ms = 0;
    bool operator==(const Identity&) const = default;
};

// An accepted Wake Signal, as handed from the channel to the application.
struct Accepted {
    Identity identity;
    int64_t expires_in_ms = 0;  // time left when it was accepted
};

// A Wake Signal the Device still owes a session for. Opening a session can fail (network,
// server), so it is retried with backoff until the signal expires. Only Settle() ends it: the
// caller settles once the server had its chance to play the Reminder. Times are monotonic.
class PendingWake {
public:
    static constexpr int64_t kRetryInitialMs = 5000;
    static constexpr int64_t kRetryMaxMs = 60000;

    // A re-delivery of the signal already pending keeps its backoff.
    void Accept(const Accepted& signal, int64_t now_ms) {
        if (pending_ && signal.identity == identity_) return;
        pending_ = true;
        identity_ = signal.identity;
        deadline_ms_ = now_ms + signal.expires_in_ms;
        next_try_ms_ = now_ms;
        backoff_ms_ = kRetryInitialMs;
    }

    bool pending() const { return pending_; }
    const Identity& identity() const { return identity_; }

    // True when a session should be opened now. An expired signal is dropped.
    bool ShouldOpen(int64_t now_ms) {
        if (!pending_) return false;
        if (now_ms >= deadline_ms_) {
            pending_ = false;
            return false;
        }
        return now_ms >= next_try_ms_;
    }

    // The session could not be opened, or dropped before the server was done with it.
    void OnSessionFailed(int64_t now_ms) {
        next_try_ms_ = now_ms + backoff_ms_;
        backoff_ms_ = backoff_ms_ * 2 > kRetryMaxMs ? kRetryMaxMs : backoff_ms_ * 2;
    }

    void Settle() { pending_ = false; }

private:
    bool pending_ = false;
    Identity identity_;
    int64_t deadline_ms_ = 0;
    int64_t next_try_ms_ = 0;
    int64_t backoff_ms_ = kRetryInitialMs;
};

}  // namespace wake_signal

#endif  // PUSH_WAKE_SIGNAL_H
