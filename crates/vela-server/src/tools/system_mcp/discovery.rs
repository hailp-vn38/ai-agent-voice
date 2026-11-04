use super::*;
impl SystemMcp {
    pub(super) async fn discover(
        &self,
        ctx: &SystemToolContext,
        arguments: Value,
    ) -> Result<Value, SystemToolError> {
        let args: Server = parse(arguments)?;
        validate_key(&args.server_key)?;
        let db = self
            .status
            .database
            .as_ref()
            .ok_or(SystemToolError::Unavailable)?;
        {
            let _publication = db.tool_security.publication.read().await;
            self.authorize(ctx, "mcp_discover_tools").await?;
        }
        let (manager, secrets) = self
            .discovery
            .as_ref()
            .ok_or(SystemToolError::Unavailable)?;
        let (source, tools) = crate::tools::external_mcp::diagnostic::discover_saved(
            db,
            manager.clone(),
            secrets.clone(),
            &args.server_key,
        )
        .await
        .map_err(|_| SystemToolError::Unavailable)?;
        let _publication = db.tool_security.publication.write().await;
        self.authorize(ctx, "mcp_discover_tools").await?;
        if ctx.cancellation.is_cancelled() || Instant::now() >= ctx.deadline {
            return Err(SystemToolError::Cancelled);
        }
        if mcp_by(db, &source.key).await?.revision != source.revision {
            return Err(SystemToolError::Denied);
        }
        db.observe_external_catalog_published(0, &source, &tools)
            .await?;
        Ok(json!({"server_key":source.key,"complete":true,"observed_tools":tools.len()}))
    }
}
