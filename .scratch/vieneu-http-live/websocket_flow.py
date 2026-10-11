import asyncio
import ctypes
import json
import time
import wave
from pathlib import Path

import websockets

OUT = Path(__file__).resolve().parent

async def main():
    events, packets = [], []
    headers = {'Authorization': 'Bearer docker-test-voice', 'Protocol-Version': '1',
               'Device-Id': 'vieneu-live-client', 'Client-Id': 'vieneu-live-validation'}
    async with websockets.connect('ws://127.0.0.1:18091/voice/v1/', additional_headers=headers) as ws:
        await ws.send(json.dumps({'type': 'hello', 'version': 1, 'transport': 'websocket',
                                 'features': {'mcp': False}, 'audio_params': {'format': 'opus',
                                 'sample_rate': 16000, 'channels': 1, 'frame_duration': 60}}))
        hello = json.loads(await asyncio.wait_for(ws.recv(), 20))
        assert hello['type'] == 'hello', hello
        events.append(hello)
        session = hello['session_id']
        await ws.send(json.dumps({'type': 'listen', 'session_id': session,
                                 'state': 'start', 'mode': 'manual'}))
        started = time.monotonic()
        await ws.send(json.dumps({'type': 'listen', 'session_id': session, 'state': 'detect',
                                 'text': 'Xin chào, hãy đọc một câu để kiểm tra giọng nói.'}))
        first_packet_ms = None
        async with asyncio.timeout(120):
            while True:
                frame = await ws.recv()
                if isinstance(frame, bytes):
                    if first_packet_ms is None:
                        first_packet_ms = (time.monotonic() - started) * 1000
                    packets.append(frame)
                else:
                    event = json.loads(frame)
                    events.append(event)
                    if event.get('type') == 'tts' and event.get('state') == 'stop':
                        break
        elapsed_ms = (time.monotonic() - started) * 1000
    assert packets, events
    assert any(e.get('type') == 'tts' and e.get('state') == 'start' for e in events), events
    opus = ctypes.CDLL('/mnt/storage/ai-agent-voice/runtime/vieneu-live/libopus.so')
    opus.opus_decoder_create.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.POINTER(ctypes.c_int)]
    opus.opus_decoder_create.restype = ctypes.c_void_p
    opus.opus_decode.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int,
                                ctypes.POINTER(ctypes.c_int16), ctypes.c_int, ctypes.c_int]
    opus.opus_decoder_destroy.argtypes = [ctypes.c_void_p]
    error = ctypes.c_int()
    decoder = opus.opus_decoder_create(24000, 1, ctypes.byref(error))
    assert decoder and error.value == 0
    decoded = bytearray()
    try:
        for packet in packets:
            pcm = (ctypes.c_int16 * 2880)()
            count = opus.opus_decode(decoder, packet, len(packet), pcm, 2880, 0)
            assert count > 0, count
            decoded.extend(bytes(pcm)[:count * 2])
    finally:
        opus.opus_decoder_destroy(decoder)
    assert any(decoded), 'Decoded audio must contain sound'
    with wave.open(str(OUT / 'vieneu-websocket.wav'), 'wb') as wav:
        wav.setparams((1, 2, 24000, 0, 'NONE', 'not compressed'))
        wav.writeframes(decoded)
    result = {'status': 'passed', 'flow': 'listen.detect -> fixture LLM -> real VieNeu HTTP streaming -> Opus -> WebSocket client -> decoded WAV',
              'first_packet_ms': first_packet_ms, 'elapsed_ms': elapsed_ms,
              'packet_count': len(packets), 'opus_bytes': sum(map(len, packets)),
              'decoded_audio_seconds': len(decoded) / 2 / 24000, 'events': events}
    (OUT / 'websocket-flow.json').write_text(json.dumps(result, ensure_ascii=False, indent=2))
    print(json.dumps(result, ensure_ascii=False))

asyncio.run(main())
