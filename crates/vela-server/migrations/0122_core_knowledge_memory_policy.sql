-- Default deny for private memory and knowledge Job tools; existing groups unchanged.
INSERT INTO core_plugin_settings(plugin_id,enabled,revision,updated_at)
VALUES ('core.knowledge',0,1,0),('core.memory',0,1,0);
