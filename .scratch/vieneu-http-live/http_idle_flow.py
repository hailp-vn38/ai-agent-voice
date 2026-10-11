import io
import json
import time
import urllib.request
import wave
from pathlib import Path

out = Path(__file__).resolve().parent
provider = json.loads((out / 'provider-created.json').read_text())
results = []
for iteration in range(3):
    request = urllib.request.Request(
        'http://127.0.0.1:18091/api/admin/providers/' + provider['key'] + '/test/tts',
        data=json.dumps({'text': 'Xin chào, đây là bài kiểm tra kết nối của VieNeu.',
                         'voice': 'Mai Anh', 'language': 'vi-VN'}).encode(),
        headers={'Authorization': 'Bearer docker-test-admin', 'Content-Type': 'application/json'},
    )
    started = time.monotonic()
    with urllib.request.urlopen(request, timeout=120) as response:
        audio = response.read()
        assert response.status == 200
        assert response.headers['x-provider-runtime-matches-desired'] == 'true'
    with wave.open(io.BytesIO(audio)) as wav:
        assert wav.getframerate() == 24000 and wav.getnchannels() == 1
        assert wav.getsampwidth() == 2 and wav.getnframes() > 0
        duration = wav.getnframes() / wav.getframerate()
    result = {'iteration': iteration + 1, 'status': 'passed',
              'elapsed_ms': (time.monotonic() - started) * 1000,
              'audio_seconds': duration, 'bytes': len(audio)}
    results.append(result)
    print(json.dumps(result), flush=True)
    if iteration < 2:
        time.sleep(6)
(out / 'http-idle-flow.json').write_text(json.dumps({
    'status': 'passed', 'idle_between_requests_seconds': 6,
    'matching_voice_and_language_overrides': True, 'runs': results,
}, indent=2))
