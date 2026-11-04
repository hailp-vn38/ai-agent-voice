-- The Plugin Store no longer approves outbound hosts or sandbox permissions.
-- Installed packages and in-flight job snapshots from the old contract are unsupported.
ALTER TABLE plugin_installations DROP COLUMN approved_permissions_json;
