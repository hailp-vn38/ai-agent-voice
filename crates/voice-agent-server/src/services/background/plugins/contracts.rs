use super::*;
impl PluginStore {
    pub(crate) async fn operation_contract(
        &self,
        choice: &FrozenChoice,
    ) -> Result<serde_json::Value, PluginError> {
        let row = self.row(&choice.plugin_id).await?;
        if row.status != "ready"
            || row.version != choice.version
            || row.package_digest != choice.package_digest
            || row.image_digest != choice.image_digest
        {
            return Err(PluginError::PermissionRequired);
        }
        let bytes = tokio::fs::read(
            self.root
                .join("packages")
                .join(&choice.plugin_id)
                .join(&choice.version)
                .join(&choice.package_digest)
                .join("package.zip"),
        )
        .await
        .map_err(|_| PluginError::StorageUnavailable)?;
        let package = archive::validate(&bytes)?;
        if package.digest != choice.package_digest || package.manifest.plugin.id != choice.plugin_id
        {
            return Err(PluginError::InvalidPackage);
        }
        let operation = package
            .manifest
            .operations
            .get(&choice.operation)
            .ok_or(PluginError::PermissionRequired)?;
        let read = |path: &str| {
            package
                .files
                .iter()
                .find(|(name, _)| name == path)
                .map(|(_, bytes)| bytes.as_slice())
                .ok_or(PluginError::InvalidPackage)
        };
        let input_schema: serde_json::Value =
            serde_json::from_slice(read(&operation.input_schema)?)
                .map_err(|_| PluginError::InvalidPackage)?;
        let output_schema: serde_json::Value =
            serde_json::from_slice(read(&operation.output_schema)?)
                .map_err(|_| PluginError::InvalidPackage)?;
        let instructions = std::str::from_utf8(read(&package.manifest.agent.instructions)?)
            .map_err(|_| PluginError::InvalidPackage)?;
        if instructions.len() > 16384 {
            return Err(PluginError::InvalidPackage);
        }
        Ok(
            json!({"description":operation.description,"instructions":instructions,"input_schema":input_schema,"output_schema":output_schema,"provides":operation.provides,"sources":operation.sources}),
        )
    }
}
