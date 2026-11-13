# Admin System API

## `GET /api/admin/system/live`

Opens an authenticated Server-Sent Events stream. Send `Accept: text/event-stream` and the normal Admin bearer token. The response sets `Cache-Control: no-store` and sends a keepalive comment every 15 seconds.

The first event is named `snapshot` and is sent immediately. Every event contains the complete latest snapshot:

```json
{
  "v": 1,
  "resources": null,
  "status": {
    "checked_at": null,
    "refresh_interval_ms": 15000,
    "summary": null
  }
}
```

`resources` is the latest System Resources sample, or `null` before a sample is available or after it ages out. `status.summary` is the safe aggregate returned by `GET /api/admin/system`; it is `null` until the first status refresh or when the refresh fails. `checked_at` records the most recent refresh attempt. The stream ends when the server shuts down. Admin session revocation is enforced by the shared Admin stream guard.

The existing `GET /api/admin/system` and `GET /api/admin/system/resources` contracts remain independent and unchanged.
