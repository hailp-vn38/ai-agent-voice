---
name: batch-source-collection
description: "Workflow for saving many YouTube videos or documents into sources/ one at a time, in small batches, using the collection ledger so work survives restarts and retries."
requires_tools: collection_status, add_to_sources
---

# Batch Source Collection Skill (`batch-source-collection`)

Use this skill when asked to save a playlist or several documents into `sources/`. Only the raw content goes into `sources/`; `wiki_ingestor` analyses each source later, exactly once.

## Find out what is left

Do not rely on your memory of the list, because old context gets condensed:

1. Call `collection_status(playlist_url=..., limit=12)`.
2. If it answers `UNKNOWN_COLLECTION`, call `yt_list_playlist(url=...)` once, then call `collection_status` again.
3. If the root gave you specific `video_id`s, process those instead.

## Process one item at a time

For each video in `NEXT`, in order:

1. Call `add_to_sources(topic_folder="<domain>/<topic>", video_id="...", title="...", tags=[...])`. It fetches the transcript itself, so do not call `yt_fetch_transcript` first.
2. Handle the result, then continue with the next video:

| Result | Meaning | What to do |
| --- | --- | --- |
| `RAW_SOURCE_SAVED` | Saved | Count as `saved` |
| `SOURCE_ALREADY_EXISTS`, `already_ingested` | Already in the Wiki | Count as `already_present` |
| `SOURCE_ALREADY_EXISTS`, `pending_ingest` | Saved, waiting for ingest | Count as `already_present` |
| `SOURCE_ALREADY_EXISTS`, `duplicate_of` | Same content as another source | Count as `duplicate` |
| Error `NO_TRANSCRIPT` | No subtitles | Count as `no_transcript` |
| Other error | Temporary failure, recorded in the ledger | Count as `failed`; do not retry it in this session |

Rules:

- One failed video never stops the batch.
- Do not write your own analysis into `markdown_body` for a video. Do not call `wiki_*` write tools.
- Pass `force=true` only when the user explicitly asked to redo that source.
- Stop after at most 12 videos, even if you could do more. A new session will continue.

## Documents

The same rules apply to PDF and Office files: one file at a time through `add_to_sources`. A file with the same content as an existing source reports `SOURCE_ALREADY_EXISTS`. Files have no ledger, so list the files you were given that are still unprocessed in your report.

## Report back

End with one line per count and the ledger totals from a final `collection_status` call:

```text
saved=8 already_present=3 duplicate=0 no_transcript=1 failed=0
TOTAL: 100 | IN_SOURCES: 52 | DUPLICATE: 0 | REMAINING: 47 | NO_TRANSCRIPT: 1 | GAVE_UP: 0
topic_folder=<domain>/<topic>
```

Keep the report short: no transcripts and no per-video summaries, because the root collects one report per batch.
