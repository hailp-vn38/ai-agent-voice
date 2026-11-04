-- Core plugin is compiled into the server; its executable dependencies are provisioned separately.
-- Preserve existing administrators' enabled/disabled settings across upgrades.
INSERT INTO core_plugin_settings(plugin_id, enabled, revision, updated_at)
VALUES ('core.yt_dlp', 1, 1, 0);
