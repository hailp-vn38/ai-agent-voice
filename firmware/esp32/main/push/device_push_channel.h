#ifndef DEVICE_PUSH_CHANNEL_H
#define DEVICE_PUSH_CHANNEL_H

#include <atomic>
#include <functional>
#include <memory>
#include <string>
#include <cstdint>

// Device Push Channel: a long-lived MQTT subscription to this Device's `signal` topic.
// It is independent of the voice protocol (own client, own settings namespace "push").
// Connection runs in its own task so Start()/Stop() never block the main loop; the
// task reconnects with exponential backoff until Stop().
class DevicePushChannel {
public:
    // Called from the MQTT task with a validated, unexpired Wake Signal. Return true if the
    // signal was acted on (so a retained replay after re-subscribe can be ignored).
    using WakeHandler = std::function<bool()>;
    // Asked before acting: true while a voice session exists.
    using SessionActiveFn = std::function<bool()>;

    DevicePushChannel(WakeHandler on_wake, SessionActiveFn session_active);
    ~DevicePushChannel();

    // True when the OTA response granted a push block (endpoint + topic present).
    static bool HasConfig();

    void Start();  // idempotent
    void Stop();   // idempotent, non-blocking

private:
    struct Run;   // one connect/subscribe/backoff loop, owned jointly with its task
    struct Memo;  // last acted-on signal, shared across reconnects within this boot
    WakeHandler on_wake_;
    SessionActiveFn session_active_;
    std::shared_ptr<Run> run_;
    std::shared_ptr<Memo> memo_;

    static void TaskMain(void* arg);
};

#endif  // DEVICE_PUSH_CHANNEL_H
