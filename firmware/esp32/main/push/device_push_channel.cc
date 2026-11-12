#include "device_push_channel.h"

#include "board.h"
#include "settings.h"
#include "wake_signal.h"

#include <cJSON.h>
#include <esp_log.h>
#include <esp_random.h>
#include <esp_timer.h>
#include <freertos/FreeRTOS.h>
#include <freertos/event_groups.h>
#include <freertos/task.h>
#include <mqtt.h>
#include <sys/time.h>

#include <algorithm>
#include <charconv>
#include <mutex>

#define TAG "PUSH"

namespace {
constexpr EventBits_t kStopBit = 1 << 0;
constexpr EventBits_t kDisconnectedBit = 1 << 1;
constexpr int kBackoffInitialMs = 5000;
constexpr int kBackoffMaxMs = 300000;
constexpr int kDefaultKeepaliveS = 240;

int64_t NowMs() {
    struct timeval tv;
    gettimeofday(&tv, nullptr);
    return static_cast<int64_t>(tv.tv_sec) * 1000 + tv.tv_usec / 1000;
}
}  // namespace

struct DevicePushChannel::Memo {
    std::mutex mutex;
    std::string id;
    int64_t expires_at_ms = 0;
};

struct DevicePushChannel::Run {
    EventGroupHandle_t eg = xEventGroupCreate();
    std::atomic<bool> stopping{false};
    std::atomic<bool> task_done{false};
    WakeHandler on_wake;
    SessionActiveFn session_active;
    std::shared_ptr<Memo> memo;
    std::atomic<int64_t> subscribed_at_ms{0};
    ~Run() { vEventGroupDelete(eg); }
};

DevicePushChannel::DevicePushChannel(WakeHandler on_wake, SessionActiveFn session_active)
    : on_wake_(std::move(on_wake)),
      session_active_(std::move(session_active)),
      memo_(std::make_shared<Memo>()) {}

DevicePushChannel::~DevicePushChannel() { Stop(); }

bool DevicePushChannel::HasConfig() {
    Settings settings("push", false);
    return !settings.GetString("endpoint").empty() && !settings.GetString("topic").empty();
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
    run->memo = memo_;
    run_ = run;
    // The task holds its own reference; it outlives Stop() just long enough to disconnect.
    auto* ref = new std::shared_ptr<Run>(run);
    if (xTaskCreate(&DevicePushChannel::TaskMain, "push_chan", 6144, ref, 3, nullptr) != pdPASS) {
        ESP_LOGE(TAG, "Failed to create push channel task");
        delete ref;
        run_.reset();
    }
}

void DevicePushChannel::Stop() {
    if (run_ == nullptr) {
        return;
    }
    run_->stopping = true;
    xEventGroupSetBits(run_->eg, kStopBit);
    run_.reset();  // task finishes on its own and frees its Run
}

void DevicePushChannel::TaskMain(void* arg) {
    auto holder = std::unique_ptr<std::shared_ptr<Run>>(static_cast<std::shared_ptr<Run>*>(arg));
    std::shared_ptr<Run> run = *holder;
    holder.reset();

    int backoff_ms = kBackoffInitialMs;
    while (!run->stopping) {
        Settings settings("push", false);
        auto endpoint = settings.GetString("endpoint");
        auto client_id = settings.GetString("client_id");
        auto username = settings.GetString("username");
        auto password = settings.GetString("password");
        auto topic = settings.GetString("topic");
        int keepalive = settings.GetInt("keepalive", kDefaultKeepaliveS);

        // endpoint is host:port; TLS is selected by the MQTT client from port 8883.
        std::string host = endpoint;
        int port = 8883;
        auto pos = endpoint.rfind(':');
        if (pos != std::string::npos) {
            host = endpoint.substr(0, pos);
            auto port_str = endpoint.substr(pos + 1);
            int parsed = 0;
            auto [ptr, ec] =
                std::from_chars(port_str.data(), port_str.data() + port_str.size(), parsed);
            if (ec == std::errc() && ptr == port_str.data() + port_str.size() && parsed > 0 &&
                parsed <= 65535) {
                port = parsed;
            } else {
                ESP_LOGW(TAG, "Invalid port in push endpoint, using %d", port);
            }
        }

        bool connected = false;
        auto network = Board::GetInstance().GetNetwork();
        std::unique_ptr<Mqtt> mqtt = network ? network->CreateMqtt(2) : nullptr;
        if (mqtt != nullptr && !host.empty() && !topic.empty()) {
            mqtt->SetKeepAlive(keepalive);
            auto* r = run.get();
            mqtt->OnDisconnected([r]() { xEventGroupSetBits(r->eg, kDisconnectedBit); });
            mqtt->OnMessage([r, topic](const std::string& t, const std::string& payload) {
                if (t != topic || r->stopping) {
                    return;
                }
                cJSON* root = cJSON_ParseWithLength(payload.data(), payload.size());
                if (root == nullptr) {
                    ESP_LOGW(TAG, "Ignoring unparseable push payload");
                    return;
                }
                wake_signal::Fields f;
                cJSON* v = cJSON_GetObjectItem(root, "v");
                if (cJSON_IsNumber(v)) f.v = static_cast<int64_t>(v->valuedouble);
                cJSON* type = cJSON_GetObjectItem(root, "type");
                if (cJSON_IsString(type)) f.type = std::string(type->valuestring);
                cJSON* exp = cJSON_GetObjectItem(root, "expires_at");
                if (cJSON_IsNumber(exp)) f.expires_at_ms = static_cast<int64_t>(exp->valuedouble);
                cJSON* idj = cJSON_GetObjectItem(root, "id");
                std::string id = cJSON_IsString(idj) ? idj->valuestring : "";
                cJSON_Delete(root);

                const int64_t now = NowMs();
                // An unset clock yields kClockUnset: the signal is ignored, never guessed fresh.
                auto verdict = wake_signal::Evaluate(f, now, r->session_active());
                if (verdict != wake_signal::Verdict::kAccept) {
                    ESP_LOGI(TAG, "Ignoring wake signal (verdict %d)", static_cast<int>(verdict));
                    return;
                }
                {
                    std::lock_guard<std::mutex> lock(r->memo->mutex);
                    int64_t since = NowMs() - r->subscribed_at_ms.load();
                    if (wake_signal::IsRetainedReplay(id, *f.expires_at_ms, r->memo->id,
                                                      r->memo->expires_at_ms, since)) {
                        ESP_LOGI(TAG, "Ignoring retained replay of handled wake signal");
                        return;
                    }
                }
                ESP_LOGI(TAG, "Wake signal accepted (id=%s)", id.c_str());
                if (r->on_wake()) {
                    std::lock_guard<std::mutex> lock(r->memo->mutex);
                    r->memo->id = id;
                    r->memo->expires_at_ms = *f.expires_at_ms;
                }
            });

            ESP_LOGI(TAG, "Connecting to push endpoint %s:%d", host.c_str(), port);
            xEventGroupClearBits(run->eg, kDisconnectedBit);
            if (auto result = mqtt->Connect(host, port, client_id, username, password); result) {
                if (!run->stopping && mqtt->Subscribe(topic, 1)) {
                    run->subscribed_at_ms = NowMs();
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
            xEventGroupWaitBits(run->eg, kStopBit | kDisconnectedBit, pdFALSE, pdFALSE,
                                portMAX_DELAY);
        }
        mqtt.reset();  // disconnects and releases the TLS session
        if (run->stopping) {
            break;
        }
        // Exponential backoff with jitter; Stop() interrupts the wait.
        int wait_ms = backoff_ms + static_cast<int>(esp_random() % (backoff_ms / 2 + 1));
        ESP_LOGI(TAG, "Push channel retry in %d ms", wait_ms);
        xEventGroupWaitBits(run->eg, kStopBit, pdFALSE, pdFALSE, pdMS_TO_TICKS(wait_ms));
        backoff_ms = std::min(backoff_ms * 2, kBackoffMaxMs);
    }
    ESP_LOGI(TAG, "Push channel stopped");
    run.reset();
    vTaskDelete(nullptr);
}
