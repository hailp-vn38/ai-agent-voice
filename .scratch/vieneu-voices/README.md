# VieNeu remote voice selection

Spec: [spec.md](spec.md). Voice presets remain owned by the running VieNeu service;
no voice catalog is copied into the Rust provider.

In Admin at http://192.168.1.157:8080/, create or edit a VieNeu Provider Instance,
enter its base URL including `/v1`, then click **Tải danh sách giọng**. Choose the
voice and save. The TTS diagnostic panel has the same discovery action and allows
a temporary voice choice without changing the saved configuration.

Editing reuses the saved credential with a revision check. Inline keys follow the
existing browser HTTPS/localhost transport rule; saved references contain no key.
If the service is unavailable, the form still accepts the exact voice name.

The existing adapter discovery endpoint now supports VieNeu remote voice metadata:

```json
{
  "selection": { "base_url": "http://vieneu:8000/v1" },
  "saved_credential": { "key": "saved-provider-key", "expected_revision": 1 }
}
```

Send it to `POST /api/admin/provider-adapters/vieneu_http/capabilities/discover`.
Use a separate write-only `api_key` instead of `saved_credential` for an unsaved
provider if needed. Discovery does not load an inference runtime or persist data;
it rejects redirects and bounds duration, response bytes, voice count and labels.

`live-flow.py` checks the running LAN Docker deployment using the existing
diagnostic Provider Instance: saved credential discovery, stale revision refusal,
real synthesis with Trúc Ly, and unchanged desired configuration/revision. It
writes `live-flow.json` and `truc-ly.wav`; it reads tokens only from local ignored
configuration and never prints them.
