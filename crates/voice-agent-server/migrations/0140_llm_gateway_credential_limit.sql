-- 0140_llm_gateway_credential_limit.sql
-- Mở rộng giới hạn kích thước credential_json của llm_gateway_accounts lên 65536 bytes
-- để lưu trữ trọn vẹn các gói session cookie lớn (như Muse AI hatch_sess / hatch_gw / hatch_vml).

CREATE TABLE llm_gateway_accounts_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_key TEXT NOT NULL UNIQUE CHECK (length(account_key) BETWEEN 3 AND 128),
    vendor TEXT NOT NULL CHECK (
        vendor IN ('anthropic', 'openai_codex', 'gemini', 'github_copilot', 'muse')
    ),
    label TEXT NOT NULL CHECK (length(label) BETWEEN 1 AND 256),
    account_email TEXT CHECK (account_email IS NULL OR length(account_email) <= 320),
    credential_json TEXT NOT NULL CHECK (length(credential_json) <= 65536),
    expires_at INTEGER,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    priority INTEGER NOT NULL DEFAULT 100 CHECK (priority BETWEEN 0 AND 10000),
    status TEXT NOT NULL DEFAULT 'active' CHECK (
        status IN ('active', 'expired', 'reauth_required', 'disabled')
    ),
    last_error TEXT CHECK (last_error IS NULL OR length(last_error) <= 1024),
    last_refreshed_at INTEGER,
    last_used_at INTEGER,
    created_at INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0
);

INSERT INTO llm_gateway_accounts_new (
    id,
    account_key,
    vendor,
    label,
    account_email,
    credential_json,
    expires_at,
    enabled,
    priority,
    status,
    last_error,
    last_refreshed_at,
    last_used_at,
    created_at,
    updated_at
)
SELECT
    id,
    account_key,
    vendor,
    label,
    account_email,
    credential_json,
    expires_at,
    enabled,
    priority,
    status,
    last_error,
    last_refreshed_at,
    last_used_at,
    created_at,
    updated_at
FROM llm_gateway_accounts;

DROP TABLE llm_gateway_accounts;
ALTER TABLE llm_gateway_accounts_new RENAME TO llm_gateway_accounts;

CREATE INDEX IF NOT EXISTS idx_llm_gateway_accounts_vendor_enabled
    ON llm_gateway_accounts (vendor, enabled, priority ASC, id ASC);
