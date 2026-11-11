# Admin Wiki Git sync API

These routes use the Admin bearer token and are served below `/api/admin`.

## `GET /knowledge/sync`

Returns the current `wiki_core::SyncStatus` plus safe server configuration. `remote_url`, `branch`, `auto`, and `interval_min` are shown; SSH key and known-host paths are never returned. When `[wiki.sync].enabled` is false, status is `{"type":"disabled","reason":"not_configured"}`.

```json
{
  "status": { "type": "idle", "last_sync": null, "ahead": 0, "behind": 0 },
  "configuration": {
    "remote_url": "git@github.com:owner/wiki.git",
    "branch": "main",
    "auto": true,
    "interval_min": 5
  }
}
```

## `POST /knowledge/sync`

Starts one sync and returns `SyncReport` (`committed`, `pulled`, `conflicts`, `pushed`). A concurrent request returns HTTP `409` with `{ "error": { "code": "sync_in_progress", "request_id": "..." } }`. Disabled sync returns `409 sync_disabled`. Git failures return a typed `sync_*` error code; raw Git stderr is not included. Authentication and transport errors use the normal Admin route behavior (`401`/`403`).

## SSE: `GET /knowledge/events`

Alongside `ready` and `knowledge_changed`, the stream sends `sync_status` with the serialized `SyncStatus` whenever sync state changes. Reconnecting clients should refresh status using the GET route.
