-- Admin Password login state: a singleton row holding the Password Login switch, the PBKDF2
-- hash of the Admin Password (empty until the server seeds the default), the forced-change flag
-- and the revision that Admin Web Tokens are pinned to. The signing secret is never stored here.
CREATE TABLE admin_auth (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    password_login_enabled INTEGER NOT NULL DEFAULT 1 CHECK (password_login_enabled IN (0, 1)),
    password_hash TEXT NOT NULL DEFAULT '',
    must_change_password INTEGER NOT NULL DEFAULT 0 CHECK (must_change_password IN (0, 1)),
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
    updated_at INTEGER NOT NULL DEFAULT 0
);

INSERT INTO admin_auth (id, password_login_enabled, password_hash, must_change_password, revision, updated_at)
VALUES (1, 1, '', 0, 1, 0);
