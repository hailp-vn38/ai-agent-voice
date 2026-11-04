pub mod adapters;
pub mod bridge;
mod dispatcher;
mod store;
pub mod types;

#[cfg(test)]
mod tests;

pub use adapters::{
    ChannelDeliveryError, ExternalDeliveryReceipt, OutboundChannelAdapter, OutboundMessagePayload,
    RecordedDelivery, RecordingChannelAdapter, ResolvedChannelTarget, TelegramBotAdapter,
    ZaloBotAdapter, default_outbound_adapters,
};
pub use types::{
    ChannelDeliveryStatusView, DeleteInboxOutput, ExternalChannelKind, InboxListQuery,
    InboxPageView, MarkReadOutput, NotificationContent, NotificationHubError, NotificationInput,
    NotificationItemView, NotificationSettingsUpdate, NotificationSettingsView,
    NotificationSeverity, NotificationSourceKind, NotificationSseEvent, NotificationTopic,
    OutboxDeliveryStatus, TelegramChannelSettingsUpdate, TelegramChannelSettingsView,
    TestChannelInput, TestChannelOutput, WebUiChannelSettingsUpdate, WebUiChannelSettingsView,
    ZaloChannelSettingsUpdate, ZaloChannelSettingsView,
};

use crate::database::{Database, secrets::SecretResolver};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::{Notify, broadcast};
use tokio_util::sync::CancellationToken;

pub struct NotificationHub {
    pub(crate) database: Database,
    pub(crate) secret_resolver: Arc<dyn SecretResolver>,
    pub(crate) adapters: HashMap<ExternalChannelKind, Arc<dyn OutboundChannelAdapter>>,
    pub(crate) web_tx: broadcast::Sender<NotificationSseEvent>,
    pub(crate) outbox_wake: Arc<Notify>,
    workers_started: AtomicBool,
}

impl std::fmt::Debug for NotificationHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotificationHub").finish_non_exhaustive()
    }
}

impl NotificationHub {
    pub fn new(
        database: Database,
        secret_resolver: Arc<dyn SecretResolver>,
        adapters: Vec<Arc<dyn OutboundChannelAdapter>>,
    ) -> Arc<Self> {
        let mut adapter_map = HashMap::new();
        for adapter in adapters {
            adapter_map.insert(adapter.channel_kind(), adapter);
        }
        let (web_tx, _) = broadcast::channel(256);
        Arc::new(Self {
            database,
            secret_resolver,
            adapters: adapter_map,
            web_tx,
            outbox_wake: Arc::new(Notify::new()),
            workers_started: AtomicBool::new(false),
        })
    }

    pub fn with_default_adapters(
        database: Database,
        secret_resolver: Arc<dyn SecretResolver>,
    ) -> Arc<Self> {
        Self::new(database, secret_resolver, default_outbound_adapters())
    }

    pub async fn publish(
        &self,
        input: NotificationInput,
    ) -> Result<NotificationItemView, NotificationHubError> {
        let (item, created) = store::publish_to_store(&self.database, input).await?;
        if created {
            let _ = self
                .web_tx
                .send(NotificationSseEvent::NotificationCreated(Box::new(
                    item.clone(),
                )));
            if !item.deliveries.is_empty() {
                self.outbox_wake.notify_one();
            }
        }
        Ok(item)
    }

    pub fn subscribe_web(&self) -> broadcast::Receiver<NotificationSseEvent> {
        self.web_tx.subscribe()
    }

    pub async fn list_inbox(
        &self,
        query: &InboxListQuery,
    ) -> Result<InboxPageView, NotificationHubError> {
        store::list_inbox_from_store(&self.database, query).await
    }

    pub async fn mark_read(
        &self,
        ids: &[String],
        mark_all: bool,
    ) -> Result<MarkReadOutput, NotificationHubError> {
        let out = store::mark_read_in_store(&self.database, ids, mark_all).await?;
        if out.updated > 0 {
            let _ = self.web_tx.send(NotificationSseEvent::UnreadCountChanged {
                unread_count: out.unread_count,
            });
        }
        Ok(out)
    }

    pub async fn delete_inbox(
        &self,
        id: Option<&str>,
        read_only: bool,
    ) -> Result<DeleteInboxOutput, NotificationHubError> {
        let out = store::delete_inbox_in_store(&self.database, id, read_only).await?;
        if out.deleted > 0 {
            let _ = self.web_tx.send(NotificationSseEvent::UnreadCountChanged {
                unread_count: out.unread_count,
            });
        }
        Ok(out)
    }

    pub async fn get_settings(&self) -> Result<NotificationSettingsView, NotificationHubError> {
        store::get_settings_from_store(&self.database, self.secret_resolver.as_ref()).await
    }

    pub async fn update_settings(
        &self,
        expected_revision: i64,
        update: NotificationSettingsUpdate,
        request_id: &str,
    ) -> Result<NotificationSettingsView, NotificationHubError> {
        store::update_settings_in_store(
            &self.database,
            self.secret_resolver.as_ref(),
            expected_revision,
            update,
            request_id,
        )
        .await
    }

    pub async fn test_channel(
        &self,
        channel: ExternalChannelKind,
        input: TestChannelInput,
    ) -> Result<TestChannelOutput, NotificationHubError> {
        dispatcher::test_channel_delivery(self, channel, input).await
    }

    pub async fn process_due_outbox(&self) -> Result<usize, NotificationHubError> {
        dispatcher::process_due_outbox(self).await
    }

    pub fn start_workers(self: &Arc<Self>, stopping: CancellationToken) {
        if self.workers_started.swap(true, Ordering::AcqRel) {
            return;
        }
        dispatcher::spawn_worker_loop(self.clone(), stopping);
    }
}
