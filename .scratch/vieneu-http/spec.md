# VieNeu HTTP Streaming Provider

Add Docker Compose for VieNeu-TTS and a server HTTP streaming TTS Provider Adapter that uses its API. User authorized the service approach superseding ADR 0027 for this adapter, requested Implement and Domain Modeling skills, including review and a commit on the current branch.

- Add an optional pinned-source CPU Compose service on the production network, with a retained model cache and healthcheck; keep existing provider defaults.
- Register `vieneu_http` through typed config, factory, loader and Admin desired-config validation so deployment and database instances work.
- Deliver raw PCM streaming from `/v1/audio/speech` into the existing TTS worker and SpeechOutput contracts. Preserve split PCM16 samples, require 24 kHz mono PCM, reject malformed/empty/excessive output and HTTP/transport failures; support total timeout and cancellation without retry.
- Support optional bearer credentials through the existing credential storage and Admin create/edit/draft forms. Do not persist or expose plaintext credentials in desired config.
- Document usage and the updated domain/architecture decision. Verify the HTTP and config boundaries, registry, materialization, relevant web forms, and full Rust/web suites.
- Exclude GPU deployment, voice-cloning management, a static upstream voice catalog and native VieNeu inference from this change.
