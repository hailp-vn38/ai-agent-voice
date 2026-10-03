# Vietnamese enrollment prompts

The WS onboarding path needs `vi-VN/intro.wav` and `vi-VN/0.wav` through `9.wav`.
Prepare them before enabling enrollment with `transport="websocket"`:

```bash
python3 scripts/prepare-enrollment-assets.py
```

The preparation tool uses locally installed `espeak-ng` (Vietnamese voice) and
`ffmpeg`. No provider credential or network service is used. Generated speech is
project-owned output; no reference-server recordings are copied. Listen to the
clips before deployment, or replace them with your own recordings.

Each file must be signed PCM16 mono 24 kHz WAV, at most 2 MiB, nonempty and not
entirely silent. Intro is at most 6 seconds; each digit is at most 1 second.
Runtime caches the PCM once before bind, assembles intro + six digits with gaps,
and encodes one continuous raw Opus stream per playback. Missing/invalid files
fail startup. Test fixtures are synthetic PCM, not the deployment voice.

Assets are deployment inputs like model artifacts. Keep the directory when
packaging a deployment, or set `prompt_assets_dir` to its absolute location.
