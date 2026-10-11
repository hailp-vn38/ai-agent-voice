# VieNeu voice selection

List voices from the configured VieNeu service so an operator can choose a preset or registered voice when creating/editing a Provider Instance and in TTS diagnostics.

- Discover the remote list without hardcoding preset names or loading inference.
- Reuse existing write-only inline or revision-checked saved credentials.
- Preserve manual voice configuration when discovery fails.
- Diagnostic overrides must use the chosen voice and leave the configured voice unchanged.
- Update the running LAN Docker deployment and verify against the real API.
