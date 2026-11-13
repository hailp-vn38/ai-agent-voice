#ifndef DEVICE_PUSH_CHANNEL_H
#define DEVICE_PUSH_CHANNEL_H

#include "wake_signal.h"

#include <cstdint>
#include <functional>
#include <memory>
#include <string>

// Device Push Channel: a long-lived MQTT subscription to this Device's `signal` topic.
// It is independent of the voice protocol (own client, own settings namespace "push").
// Connection runs in its own task so Start()/Stop() never block the main loop; the
// task reconnects with exponential backoff until Stop().
class DevicePushChannel {
public:
    // Called from the MQTT task with a validated, unexpired Wake Signal that has not been
    // handled yet. The same signal is handed over again after every re-subscribe (the broker
    // retains it) until MarkHandled() is called for it.
    using WakeHandler = std::function<void(const wake_signal::Accepted&)>;
    // Asked before acting: true while a voice session exists.
    using SessionActiveFn = std::function<bool()>;
    // Unix time in milliseconds (UTC), used to judge a signal's expiry.
    using UtcNowFn = std::function<int64_t()>;

    DevicePushChannel(WakeHandler on_wake, SessionActiveFn session_active, UtcNowFn utc_now_ms);
    ~DevicePushChannel();

    // True when the OTA response granted a push block (endpoint + topic present).
    static bool HasConfig();

    void Start();  // idempotent
    void Stop();   // idempotent, non-blocking

    // The server had its chance to act on this signal: its retained copy is ignored from now
    // on (within this boot). Thread-safe.
    void MarkHandled(const wake_signal::Identity& signal);

private:
    struct Run;            // one connect/subscribe/backoff loop, owned jointly with its task
    struct HandledSignal;  // last handled signal, shared across reconnects within this boot
    WakeHandler on_wake_;
    SessionActiveFn session_active_;
    UtcNowFn utc_now_ms_;
    std::shared_ptr<Run> run_;
    std::shared_ptr<HandledSignal> handled_;

    static void TaskMain(void* arg);
};

#endif  // DEVICE_PUSH_CHANNEL_H
