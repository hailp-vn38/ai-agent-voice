import io
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time
import urllib.request
import uuid
import wave

root = Path(__file__).resolve().parents[2]
out = Path(__file__).resolve().parent
binary = Path('/mnt/storage/ai-agent-voice/target/debug/voice-agent-server')
with tempfile.TemporaryDirectory(prefix='vieneu-native-check-', dir=root / 'runtime') as temporary:
    temporary = Path(temporary)
    token = uuid.uuid4().hex
    nonce = str(uuid.uuid4())
    config = temporary / 'config.toml'
    config.write_text(f'''[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"
[api]
enabled = true
admin_token = "{token}"
[api.provider_tests]
timeout_ms = 30000
[database]
url = "sqlite://{temporary}/voice.db"
[provider_defaults]
vad = "vad"
asr = "asr"
llm = "llm"
tts = "native"
[providers.vad.instances.vad]
adapter = "silero_onnx"
[providers.asr.instances.asr]
adapter = "gipformer_sherpa_offline"
[providers.llm.instances.llm]
adapter = "openai"
api_key = ""
model = "fixture"
[providers.tts.instances.native]
adapter = "vieneu_native"
voice = "hai_dang"
language = "vi-VN"
preload = true
[runtime.onnx]
library = "runtime/onnxruntime/libonnxruntime.so"
[runtime.onnx.threads]
vieneu_native = 2
[provider_runtime]
max_parallel_loads = 1
max_pending_loads = 8
max_waiters = 64
max_resident_bytes = 8589934592
max_resources = 16
max_version_entries = 128
admission_timeout_ms = 15000
startup_timeout_ms = 60000
failure_cooldown_ms = 1000
idle_ttl_ms = 600000
[provider_runtime.estimated_peak_bytes]
vieneu_native = 2147483648
silero_onnx = 134217728
gipformer_sherpa_offline = 536870912
openai = 67108864
''')
    config.chmod(0o600)
    address_file = temporary / 'address.json'
    env = dict(os.environ, VOICE_AGENT_CONFIG=str(config), VOICE_AGENT_BOUND_ADDRESS_FILE=str(address_file), VOICE_AGENT_STARTUP_NONCE=nonce)
    log = (out / 'server.log').open('w')
    process = subprocess.Popen([binary], cwd=root, env=env, stdout=log, stderr=subprocess.STDOUT)
    try:
        deadline = time.monotonic() + 120
        while not address_file.exists():
            assert process.poll() is None, 'server failed before listener handshake; see server.log'
            assert time.monotonic() < deadline, 'server startup timed out'
            time.sleep(0.2)
        address = json.loads(address_file.read_text())
        assert address['startup_nonce'] == nonce and address['pid'] == process.pid
        base = 'http://' + address['address']
        results = []
        for voice in ['hai_dang', 'mai_anh']:
            started = time.monotonic()
            request = urllib.request.Request(base + '/api/admin/providers/config:tts:native/test/tts',
                data=json.dumps({'text': 'Xin chào, tôi là trợ lý giọng nói tiếng Việt.', 'voice': voice, 'language': 'vi-VN'}).encode(),
                headers={'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json'})
            with urllib.request.urlopen(request, timeout=40) as response:
                assert response.status == 200
                data = response.read()
            with wave.open(io.BytesIO(data)) as wav:
                assert wav.getnchannels() == 1 and wav.getframerate() == 48000 and wav.getnframes() > 0
                duration = wav.getnframes() / wav.getframerate()
            (out / ('server-' + voice + '.wav')).write_bytes(data)
            elapsed = time.monotonic() - started
            results.append({'voice': voice, 'elapsed_s': elapsed, 'audio_s': duration, 'rtf': elapsed / duration, 'wav_bytes': len(data)})
        report = {'status': 'passed', 'flow': 'production startup -> managed deployment provider -> retained native worker -> Admin HTTP diagnostic', 'runs': results}
        (out / 'server-check.json').write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps(report))
    finally:
        process.send_signal(signal.SIGINT)
        try:
            process.wait(timeout=20)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        log.close()
