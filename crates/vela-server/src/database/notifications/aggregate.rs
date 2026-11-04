use super::*;
use sqlx::{QueryBuilder, Sqlite};
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentReminderQuery {
    pub device_id: Option<String>,
    pub status: Option<String>,
    pub due_from_ms: Option<i64>,
    pub due_to_ms: Option<i64>,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub device_page: Option<u32>,
}
#[derive(Serialize, FromRow)]
struct PickerDevice {
    device_id: String,
    name: Option<String>,
    enabled: bool,
}
fn filter<'a>(
    query: &mut QueryBuilder<'a, Sqlite>,
    key: &'a str,
    input: &'a AgentReminderQuery,
    with_status: bool,
) {
    query.push(" FROM device_reminders r JOIN devices d ON d.id=r.device_row_id JOIN agents a ON a.id=d.agent_id WHERE a.key=").push_bind(key).push(" AND d.kind='physical'");
    if let Some(device) = &input.device_id {
        query.push(" AND d.device_id=").push_bind(device);
    }
    if with_status && let Some(status) = &input.status {
        query.push(" AND r.status=").push_bind(status);
    }
    if let Some(from) = input.due_from_ms {
        query.push(" AND r.due_at_ms>=").push_bind(from);
    }
    if let Some(to) = input.due_to_ms {
        query.push(" AND r.due_at_ms<=").push_bind(to);
    }
}
impl DeviceNotifications {
    pub async fn list_agent(
        &self,
        key: &str,
        input: &AgentReminderQuery,
    ) -> Result<serde_json::Value, NotificationError> {
        let page = input.page.unwrap_or(1);
        let size = input.page_size.unwrap_or(50);
        let device_page = input.device_page.unwrap_or(1);
        if page == 0
            || device_page == 0
            || !(1..=200).contains(&size)
            || input
                .status
                .as_deref()
                .is_some_and(|v| !matches!(v, "pending" | "completed" | "cancelled" | "expired"))
            || matches!((input.due_from_ms,input.due_to_ms),(Some(from),Some(to)) if from>to)
        {
            return Err(NotificationError::Invalid("invalid_query"));
        }
        let mut tx = self.database.pool().begin().await?;
        sqlx::query_scalar::<_, i64>("SELECT id FROM agents WHERE key=?")
            .bind(key)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(NotificationError::NotFound)?;
        if let Some(device) = &input.device_id {
            sqlx::query_scalar::<_,i64>("SELECT d.id FROM devices d JOIN agents a ON a.id=d.agent_id WHERE a.key=? AND d.device_id=? AND d.kind='physical'").bind(key).bind(device).fetch_optional(&mut *tx).await?.ok_or(NotificationError::NotFound)?;
        }
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT r.id,d.device_id,r.message,r.due_at_ms,r.timezone,r.status,r.row_revision,r.schedule_revision,r.attempts_used,r.created_at_ms,r.updated_at_ms",
        );
        filter(&mut query, key, input, true);
        query
            .push(" ORDER BY r.due_at_ms,r.id LIMIT ")
            .push_bind(size)
            .push(" OFFSET ")
            .push_bind(i64::from(page - 1) * i64::from(size));
        let mut items: Vec<Reminder> = query.build_query_as().fetch_all(&mut *tx).await?;
        let mut device_rows = std::collections::HashMap::<String, i64>::new();
        for item in &mut items {
            let row_id = if let Some(&id) = device_rows.get(&item.device_id) {
                id
            } else {
                let id: i64 = sqlx::query_scalar("SELECT id FROM devices WHERE device_id=?")
                    .bind(&item.device_id)
                    .fetch_one(&mut *tx)
                    .await?;
                device_rows.insert(item.device_id.clone(), id);
                id
            };
            item.sync_status = sync_status(&mut tx, row_id, item).await?;
        }
        let mut count = QueryBuilder::<Sqlite>::new("SELECT count(*)");
        filter(&mut count, key, input, true);
        let total: i64 = count.build_query_scalar().fetch_one(&mut *tx).await?;
        let mut counts = QueryBuilder::<Sqlite>::new("SELECT r.status,count(*)");
        filter(&mut counts, key, input, false);
        counts.push(" GROUP BY r.status");
        let rows: Vec<(String, i64)> = counts.build_query_as().fetch_all(&mut *tx).await?;
        let mut kpi = serde_json::json!({"pending":0,"completed":0,"cancelled":0,"expired":0});
        for (status, n) in rows {
            kpi[status] = n.into();
        }
        let devices:Vec<PickerDevice>=sqlx::query_as("SELECT d.device_id,d.name,d.enabled FROM devices d JOIN agents a ON a.id=d.agent_id WHERE a.key=? AND d.kind='physical' ORDER BY d.device_id LIMIT 201 OFFSET ?").bind(key).bind(i64::from(device_page-1)*200).fetch_all(&mut *tx).await?;
        let device_has_more = devices.len() > 200;
        tx.commit().await?;
        Ok(
            serde_json::json!({"items":items,"total":total,"page":page,"page_size":size,"has_more":i64::from(page)*i64::from(size)<total,"kpi":kpi,"devices":devices.into_iter().take(200).collect::<Vec<_>>(),"device_page":device_page,"device_has_more":device_has_more}),
        )
    }
}
