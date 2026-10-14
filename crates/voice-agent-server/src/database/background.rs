use super::Database;
use crate::services::background::{BackgroundSettingsUpdate, JobError};

impl Database {
    pub(crate) async fn disable_background_execution(&self) -> Result<(), JobError> {
        let _publication = self.tool_security.publication.write().await;
        sqlx::query("UPDATE background_settings SET enabled=0,revision=revision+1 WHERE singleton=1 AND enabled=1")
            .execute(self.pool()).await?;
        Ok(())
    }

    pub(crate) async fn background_settings(
        &self,
    ) -> Result<(BackgroundSettingsUpdate, i64), JobError> {
        let (enabled,orchestrator_llm_provider,worker_llm_provider,revision): (i64,Option<String>,Option<String>,i64) =
            sqlx::query_as("SELECT enabled,orchestrator_llm_provider,worker_llm_provider,revision FROM background_settings WHERE singleton=1")
                .fetch_one(self.pool()).await?;
        Ok((
            BackgroundSettingsUpdate {
                enabled: enabled == 1,
                orchestrator_llm_provider,
                worker_llm_provider,
            },
            revision,
        ))
    }
    pub(crate) async fn save_background_settings(
        &self,
        input: &BackgroundSettingsUpdate,
        expected: i64,
    ) -> Result<(), JobError> {
        let _publication = self.tool_security.publication.write().await;
        let changed = sqlx::query("UPDATE background_settings SET enabled=?,orchestrator_llm_provider=?,worker_llm_provider=?,revision=revision+1 WHERE singleton=1 AND revision=?")
            .bind(input.enabled).bind(&input.orchestrator_llm_provider).bind(&input.worker_llm_provider).bind(expected)
            .execute(self.pool()).await?.rows_affected();
        if changed != 1 {
            return Err(JobError::RevisionConflict);
        }
        Ok(())
    }
}
