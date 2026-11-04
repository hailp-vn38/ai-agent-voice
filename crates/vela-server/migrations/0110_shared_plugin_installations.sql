-- Cutover after draining/terminalizing old Job attempts. Old Docker images are unsupported.
ALTER TABLE plugin_installations RENAME COLUMN image_digest TO installation_digest;
UPDATE plugin_installations
SET status = 'disabled', installation_digest = '', revision = revision + 1,
    last_error_code = 'requires_shared_runtime_reinstall';
ALTER TABLE job_steps RENAME COLUMN image_digest TO installation_digest;
