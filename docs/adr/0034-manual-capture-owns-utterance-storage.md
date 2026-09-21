# ADR 0034 — Manual Capture sở hữu PCM của lượt thu manual

`ManualCapture` sở hữu PCM, capacity và state của lượt thu manual; `SessionActor` chỉ điều phối `listen:start`, frame đã decode và `listen:stop`, rồi nhận `CaptureOutcome`. Ranh giới này giữ actor không biết storage/cách tính duration, đồng thời cho phép Manual Capture và VAD sau này cùng tạo Uplink Audio Utterance mà không đổi trách nhiệm actor. Capture pre-reserve capacity fallible theo integer frame count khi audio runtime khởi tạo, không realloc trong khi thu; init lỗi đóng connection mới 1011 trước ServerHello.
