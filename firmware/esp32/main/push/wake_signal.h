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
    std::optional<int64_t> expires_at_ms;
};

enum class Verdict { kAccept, kWrongVersion, kWrongType, kExpired, kBusy };

inline Verdict Evaluate(const Fields& f, int64_t now_ms, bool session_active) {
    if (!f.v || *f.v != kSupportedVersion) return Verdict::kWrongVersion;
    if (!f.type || *f.type != kWakeType) return Verdict::kWrongType;
    if (!f.expires_at_ms || *f.expires_at_ms <= now_ms) return Verdict::kExpired;
    if (session_active) return Verdict::kBusy;
    return Verdict::kAccept;
}

// A broker re-delivers the retained signal on every subscribe. Once a signal has been acted on,
// the same (id, expires_at) arriving right after a re-subscribe is that replay, not a new wake.
constexpr int64_t kReplayWindowMs = 5000;

inline bool IsRetainedReplay(const std::string& id, int64_t expires_at_ms,
                             const std::string& last_id, int64_t last_expires_at_ms,
                             int64_t ms_since_subscribe) {
    return !last_id.empty() && id == last_id && expires_at_ms == last_expires_at_ms &&
           ms_since_subscribe < kReplayWindowMs;
}

}  // namespace wake_signal

#endif  // PUSH_WAKE_SIGNAL_H
