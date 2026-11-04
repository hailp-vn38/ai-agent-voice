-- Store only authenticated ciphertext and masked metadata, separate from typed configuration.
ALTER TABLE providers ADD COLUMN credential_json TEXT
    CHECK (credential_json IS NULL OR (length(CAST(credential_json AS BLOB)) <= 8192 AND json_valid(credential_json)));
ALTER TABLE mcp_servers ADD COLUMN credential_json TEXT
    CHECK (credential_json IS NULL OR (length(CAST(credential_json AS BLOB)) <= 8192 AND json_valid(credential_json)));
