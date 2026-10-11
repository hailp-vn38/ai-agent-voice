import io
import json
import time
import tomllib
import urllib.error
import urllib.request
import wave
from pathlib import Path

out = Path(__file__).resolve().parent
root = out.parents[1]
config = tomllib.loads((root / 'config.toml').read_text())
key = json.loads((root / '.scratch/docker-compose-vieneu/smoke.json').read_text())['provider_key']
base = 'http://192.168.1.157:8080/api/admin'
opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

def request(path, body=None):
    return opener.open(urllib.request.Request(base + path,
        data=None if body is None else json.dumps(body).encode(),
        headers={'Authorization': 'Bearer ' + config['api']['admin_token'],
                 'Content-Type': 'application/json'}), timeout=120)

with request('/providers/' + key) as response:
    provider = json.load(response)
selection = json.loads(provider['config_json'])
payload = {'selection': selection, 'saved_credential': {'key': key, 'expected_revision': provider['revision']}}
with request('/provider-adapters/vieneu_http/capabilities/discover', payload) as response:
    capabilities = json.load(response)
ids = [voice['id'] for voice in capabilities['voices']]
assert len(ids) == len(set(ids)) and 'Trúc Ly' in ids and 'Mai Anh' in ids
stale = {**payload, 'saved_credential': {'key': key, 'expected_revision': provider['revision'] + 1}}
try:
    request('/provider-adapters/vieneu_http/capabilities/discover', stale)
    raise AssertionError('stale credential accepted')
except urllib.error.HTTPError as error:
    assert error.code == 409
started = time.monotonic()
with request('/providers/' + key + '/test/tts', {
    'text': 'Xin chào, đây là giọng Trúc Ly từ VieNeu.', 'voice': 'Trúc Ly', 'language': 'vi-VN',
}) as response:
    assert response.status == 200
    audio = response.read()
elapsed_ms = round((time.monotonic() - started) * 1000)
with wave.open(io.BytesIO(audio)) as wav:
    assert wav.getframerate() == 24000 and wav.getnframes() > 0
    duration = wav.getnframes() / wav.getframerate()
with request('/providers/' + key) as response:
    after = json.load(response)
assert after['revision'] == provider['revision'] and after['config_json'] == provider['config_json']
(out / 'truc-ly.wav').write_bytes(audio)
report = {'status': 'passed', 'base_url': base, 'voice_count': len(ids), 'voice_ids': ids,
          'saved_credential_discovery': 'passed', 'stale_credential_rejected': 'passed',
          'diagnostic_voice': 'Trúc Ly', 'desired_config_unchanged': True,
          'sample_rate': 24000, 'audio_seconds': duration, 'elapsed_ms': elapsed_ms}
(out / 'live-flow.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
print(json.dumps(report, ensure_ascii=False, indent=2))
