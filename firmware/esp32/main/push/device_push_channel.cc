#include "device_push_channel.h"

#include "board.h"
#include "settings.h"

#include <cJSON.h>
#include <esp_log.h>
#include <esp_random.h>
#include <freertos/FreeRTOS.h>
#include <freertos/event_groups.h>
#include <freertos/task.h>
#include <mqtt.h>

#include <algorithm>
#include <atomic>
#include <charconv>
#include <mutex>
#include <optional>

#define TAG "PUSH"

namespace {
constexpr EventBits_t kStopBit = 1 << 0;
constexpr EventBits_t kDisconnectedBit = 1 << 1;
constexpr int kBackoffInitialMs = 5000;
constexpr int kBackoffMaxMs = 300000;
constexpr int kDefaultKeepaliveS = 240;
constexpr int kDefaultPort = 8883;

struct Endpoint {
    std::string host;
    int port = kDefaultPort;
};

// `host:port`; TLS is selected by the MQTT client from port 8883, which is also the default.
Endpoint ParseEndpoint(const std::string& endpoint) {
    Endpoint parsed{endpoint, kDefaultPort};
    auto colon = endpoint.rfind(':');
    if (colon == std::string::npos) {
        return parsed;
    }
    parsed.host = endpoint.substr(0, colon);
    auto port_text = endpoint.substr(colon + 1);
    int port = 0;
    auto [end, error] = std::from_chars(port_text.data(), port_text.data() + port_text.size(), port);
    if (error == std::errc() && end == port_text.data() + port_text.size() && port > 0 &&
        port <= 65535) {
        parsed.port = port;
    } else {
        ESP_LOGW(TAG, "Invalid port in push endpoint, using %d", parsed.port);
    }
    return parsed;
}

std::optional<wake_signal::Fields> ParseSignal(const std::string& payload) {
    cJSON* root = cJSON_ParseWithLength(payload.data(), payload.size());
    if (root == nullptr) {
        return std::nullopt;
    }
    auto number = [root](const char* key) -> std::optional<int64_t> {
        cJSON* item = cJSON_GetObjectItem(root, key);
        if (!cJSON_IsNumber(item)) return std::nullopt;
        return static_cast<int64_t>(item->valuedouble);
    };
    auto text = [root](const char* key) -> std::optional<std::string> {
        cJSON* item = cJSON_GetObjectItem(root, key);
        if (!cJSON_IsString(item)) return std::nullopt;
        return std::string(item->valuestring);
    };
    wake_signal::Fields fields{number("v"), text("type"), text("id"), number("at"),
                               number("expires_at")};
    cJSON_Delete(root);
    return fields;
}
}  // namespace

struct DevicePushChannel::HandledSignal {
    std::mutex mutex;
    std::optional<wake_signal::Identity> identity;
};

struct DevicePushChannel::Run {
    EventGroupHandle_t events = xEventGroupCreate();
    std::atomic<bool> stopping{false};
    WakeHandler on_wake;
    SessionActiveFn session_active;
    UtcNowFn utc_now_ms;
    std::shared_ptr<HandledSignal> handled;
    ~Run() { vEventGroupDelete(events); }

    void OnMessage(const std::string& payload) {
        auto fields = ParseSignal(payload);
        if (!fields) {
            ESP_LOGW(TAG, "Ignoring unparseable push payload");
            return;
        }
        // An unset clock yields kClockUnset: the signal is ignored, never guessed fresh.
        const int64_t utc_now = utc_now_ms();
        auto verdict = wake_signal::Evaluate(*fields, utc_now, session_active());
        if (verdict != wake_signal::Verdict::kAccept) {
            ESP_LOGI(TAG, "Ignoring wake signal (verdict %d)", static_cast<int>(verdict));
            return;
        }
        wake_signal::Identity identity{*fields->id, *fields->at_ms};
        {
            std::lock_guard<std::mutex> lock(handled->mutex);
            if (handled->identity == identity) {
                ESP_LOGI(TAG, "Ignoring retained copy of handled wake signal");
                return;
            }
        }
        ESP_LOGI(TAG, "Wake signal accepted (id=%s)", identity.id.c_str());
        on_wake(wake_signal::Accepted{identity, *fields->expires_at_ms - utc_now});
    }
};

DevicePushChannel::DevicePushChannel(WakeHandler on_wake, SessionActiveFn session_active,
                                     UtcNowFn utc_now_ms)
    : on_wake_(std::move(on_wake)),
      session_active_(std::move(session_active)),
      utc_now_ms_(std::move(utc_now_ms)),
      handled_(std::make_shared<HandledSignal>()) {}

DevicePushChannel::~DevicePushChannel() { Stop(); }

bool DevicePushChannel::HasConfig() {
    Settings settings("push", false);
    return !settings.GetString("endpoint").empty() && !settings.GetString("topic").empty();
}

void DevicePushChannel::MarkHandled(const wake_signal::Identity& signal) {
    std::lock_guard<std::mutex> lock(handled_->mutex);
    handled_->identity = signal;
}

void DevicePushChannel::Start() {
    if (run_ != nullptr) {
        return;
    }
    if (!HasConfig()) {
        return;
    }
    auto run = std::make_shared<Run>();
    run->on_wake = on_wake_;
    run->session_active = session_active_;
    run->utc_now_ms = utc_now_ms_;
    run->handled = handled_;
    run_ = run;
    // The task holds its own reference; it outlives Stop() just long enough to disconnect.
    auto* task_ref = new std::shared_ptr<Run>(run);
    if (xTaskCreate(&DevicePushChannel::TaskMain, "push_chan", 6144, task_ref, 3, nullptr) !=
        pdPASS) {
        ESP_LOGE(TAG, "Failed to create push channel task");
        delete task_ref;
        run_.reset();
    }
}

void DevicePushChannel::Stop() {
    if (run_ == nullptr) {
        return;
    }
    run_->stopping = true;
    xEventGroupSetBits(run_->events, kStopBit);
    run_.reset();  // task finishes on its own and frees its Run
}

void DevicePushChannel::TaskMain(void* arg) {
    auto holder = std::unique_ptr<std::shared_ptr<Run>>(static_cast<std::shared_ptr<Run>*>(arg));
    std::shared_ptr<Run> run = *holder;
    holder.reset();

    int backoff_ms = kBackoffInitialMs;
    while (!run->stopping) {
        Settings settings("push", false);
        auto endpoint = ParseEndpoint(settings.GetString("endpoint"));
        auto client_id = settings.GetString("client_id");
        auto username = settings.GetString("username");
        auto password = settings.GetString("password");
        auto topic = settings.GetString("topic");
        int keepalive = settings.GetInt("keepalive", kDefaultKeepaliveS);

        bool connected = false;
        auto network = Board::GetInstance().GetNetwork();
        std::unique_ptr<Mqtt> mqtt = network ? network->CreateMqtt(2) : nullptr;
        if (mqtt != nullptr && !endpoint.host.empty() && !topic.empty()) {
            mqtt->SetKeepAlive(keepalive);
            auto* self = run.get();
            mqtt->OnDisconnected([self]() { xEventGroupSetBits(self->events, kDisconnectedBit); });
            mqtt->OnMessage([self, topic](const std::string& message_topic,
                                          const std::string& payload) {
                if (message_topic == topic && !self->stopping) {
                    self->OnMessage(payload);
                }
            });

            ESP_LOGI(TAG, "Connecting to push endpoint %s:%d", endpoint.host.c_str(),
                     endpoint.port);
            xEventGroupClearBits(run->events, kDisconnectedBit);
            if (auto result =
                    mqtt->Connect(endpoint.host, endpoint.port, client_id, username, password);
                result) {
                if (!run->stopping && mqtt->Subscribe(topic, 1)) {
                    connected = true;
                    backoff_ms = kBackoffInitialMs;
                    ESP_LOGI(TAG, "Push channel connected, subscribed to %s", topic.c_str());
                } else if (!run->stopping) {
                    ESP_LOGW(TAG, "Push subscribe failed");
                }
            } else {
                ESP_LOGW(TAG, "Push connect failed: %s", result.error().ToString().c_str());
            }
        }

        if (connected) {
            xEventGroupWaitBits(run->events, kStopBit | kDisconnectedBit, pdFALSE, pdFALSE,
                                portMAX_DELAY);
        }
        mqtt.reset();  // disconnects and releases the TLS session
        if (run->stopping) {
            break;
        }
        // Exponential backoff with jitter; Stop() interrupts the wait.
        int wait_ms = backoff_ms + static_cast<int>(esp_random() % (backoff_ms / 2 + 1));
        ESP_LOGI(TAG, "Push channel retry in %d ms", wait_ms);
        xEventGroupWaitBits(run->events, kStopBit, pdFALSE, pdFALSE, pdMS_TO_TICKS(wait_ms));
        backoff_ms = std::min(backoff_ms * 2, kBackoffMaxMs);
    }
    ESP_LOGI(TAG, "Push channel stopped");
    run.reset();
    vTaskDelete(nullptr);
}
