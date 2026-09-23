# Voice Reference Client

`voice-reference-client` la client V1 toi thieu de chay mot text turn voi server. Client dung OTA discovery de lay WebSocket URL va token, gui cac header V1 bat buoc, sau do thuc hien `ClientHello`, `listen:start`, `listen:detect` va kiem tra chuoi TTS downlink.

Binary mac dinh chi phuc vu ket noi server. Binary `zerotts` rieng chay model cuc bo de do latency va nghe PCM truoc resample/Opus; no khong kiem tra giao thuc.

## Do ZeroTTS cuc bo

Can pack `models/zerotts` va `VOICE_ONNX_RUNTIME_LIB` tro den thu vien ONNX Runtime da cai. Chay tu root repo:

```bash
cargo run --release -p voice-reference-client --bin zerotts -- \
  "Xin chào, đây là phép đo ZeroTTS." \
  --model-dir models/zerotts --threads 2 --repeats 2 \
  --out /tmp/zerotts-reference.wav
```

Lenh in `init_ms`, `first_pcm_ms`, `synthesis_ms`, `audio_ms`, RTF, so chunk va so sample cua tung lan. WAV la float32 mono 48 kHz tu codec, chua qua SpeechOutput, Opus, pacing hoac WebSocket. Loop nay theo `synthesize_stream` cua Python: mot lan text encoder/prefix, AR frame, `min_frames=4`, giu frame EOA, codec chunk 1/2/4/8/16 va reset cache codec moi lan tong hop. Dau vao duoc dua truc tiep vao tokenizer; CLI Python co buoc chuan hoa tieng Viet rieng. Random draw cua binary Rust la deterministic de lap lai phep do, nen khong doi waveform bit-exact voi Python mac dinh.

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
