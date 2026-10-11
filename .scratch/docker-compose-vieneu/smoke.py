import io
import json
import time
import tomllib
import urllib.request
import wave
from pathlib import Path

root = Path(__file__).resolve().parents[2]
out = Path(__file__).resolve().parent
config = tomllib.loads((root / 'config.toml').read_text())
api_key = Path('/mnt/storage/ai-agent-voice/runtime/vieneu-live/api-key').read_text().strip()
base = 'http://127.0.0.1:8080'
headers = {'Authorization': 'Bearer ' + config['api']['admin_token'],
           'Content-Type': 'application/json'}

def request(path, body=None):
    data = None if body is None else json.dumps(body).encode()
    return urllib.request.urlopen(urllib.request.Request(base + path, data=data, headers=headers), timeout=120)

for path in ['/', '/health', '/ready']:
    with request(path) as response:
        assert response.status == 200

with request('/api/admin/providers', {
    'name': 'VieNeu Docker', 'type': 'tts', 'adapter': 'vieneu_http',
    'config_json': {'base_url': 'http://vieneu:8000/v1', 'voice': 'Mai Anh',
                    'language': 'vi-VN', 'timeout_ms': 120000}, 'api_key': api_key,
}) as response:
    body = response.read()
    assert api_key.encode() not in body
    provider = json.loads(body)

runs = []
for iteration in range(2):
    started = time.monotonic()
    with request('/api/admin/providers/' + provider['key'] + '/test/tts', {
        'text': 'Xin chào, đây là VieNeu chạy trong Docker Compose.',
        'voice': 'Mai Anh', 'language': 'vi-VN',
    }) as response:
        audio = response.read()
        assert response.status == 200
        assert response.headers['x-provider-runtime-matches-desired'] == 'true'
    with wave.open(io.BytesIO(audio)) as wav:
        assert wav.getframerate() == 24000 and wav.getnchannels() == 1
        assert wav.getsampwidth() == 2 and wav.getnframes() > 0
        duration = wav.getnframes() / wav.getframerate()
    (out / 'vieneu-docker.wav').write_bytes(audio)
    runs.append({'iteration': iteration + 1, 'elapsed_ms': round((time.monotonic() - started) * 1000),
                 'audio_seconds': duration, 'bytes': len(audio), 'sample_rate': 24000})
    if iteration == 0:
        time.sleep(6)
report = {'status': 'passed', 'provider_key': provider['key'], 'checks': ['web', 'health', 'ready',
          'sealed credential', 'real TTS', 'idle reconnect'], 'runs': runs}
(out / 'smoke.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
