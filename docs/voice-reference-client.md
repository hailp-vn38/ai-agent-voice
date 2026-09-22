# Voice Reference Client

`voice-reference-client` la client V1 toi thieu de chay mot text turn voi server. Client dung OTA discovery de lay WebSocket URL va token, gui cac header V1 bat buoc, sau do thuc hien `ClientHello`, `listen:start`, `listen:detect` va kiem tra chuoi TTS downlink.

No chi phuc vu ket noi server; khong chua local model smoke, replay WAV/Opus, protocol fixture, hay test noi bo.

## Chay voi server cuc bo

Khoi dong server o terminal khac:

```bash
VOICE_AGENT_CONFIG=config.example.toml cargo run -p voice-agent-server
```

Chay mot text turn:

```bash
cargo run -p voice-reference-client -- \
  --ota http://127.0.0.1:8000/voice/ota/ \
  "Xin chao"
```

Mac dinh `Device-Id` la `reference-client-01` va `Client-Id` la `reference-client`. Co the thay doi chung khi can:

```bash
cargo run -p voice-reference-client -- \
  --ota http://127.0.0.1:8000/voice/ota/ \
  --device-id reference-device-02 \
  --client-id reference-client-02 \
  "hãy đọc 'how are you'" \
  --debug-audio-file outpu.wav
```

Client yeu cau ServerHello va downlink Opus 24 kHz mono, 60 ms hop le; chi pass sau `tts:start`, it nhat mot binary packet va `tts:stop`. Khong in token hoac noi dung text nhan tu server.

`--debug-steps` chi ghi ten buoc an toan ve rieng tu. `--debug-audio-file path.wav` la tuy chon chan doan, ghi audio TTS da decode ra WAV sau turn thanh cong.
