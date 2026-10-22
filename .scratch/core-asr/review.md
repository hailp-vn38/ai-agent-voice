# Core ASR implementation review

Base: `acc246be000a430fee715b40a1ba2014bafbd203`.

Two independent reviewers examined the implementation against repository standards and `.scratch/core-asr/spec.md`.

## Standards

One concrete finding was fixed in `2aafa6e9`: provider validation and Desired Provider snapshot reads previously used separate autocommit reads. Concurrent mutation could materialize an unpinned revision before dispatch rejected it. ASR now reads both within one transaction; concurrent-mutation regression checks every successful snapshot retains the admitted revision and configuration. The Standards reviewer rechecked the fix and reported no outstanding findings.

The possible duplication between the ASR and TTS pin modules was considered and accepted within this feature's explicit scope: separate ASR pins are allowed, and existing TTS pins must not be redesigned solely to add ASR.

## Spec

No confirmed defects or scope creep. The reviewer checked pins/dispatch, bounded artifact input and WAV conversion, isolated events, cleanup/quarantine, complete Child output and retry authority. The existing fixed 48→24 kHz resampler cannot implement the required arbitrary 8–48→16 kHz conversion; the bounded converter is therefore necessary.

## Result

Outstanding findings: Standards **0**, Spec **0**. Review was read-only; automated check evidence and real-provider limitations are recorded in [the flow](../../docs/flows/15-core-asr-v3.md).
