//! In-process invalidation only. SQLite remains the authoritative Job read model.
use tokio::sync::broadcast;

#[derive(Clone, Debug)]
pub(crate) struct JobChangeFeed {
    sender: broadcast::Sender<String>,
}
impl Default for JobChangeFeed {
    fn default() -> Self {
        let (sender, _) = broadcast::channel(256);
        Self { sender }
    }
}
impl JobChangeFeed {
    pub(crate) fn subscribe(&self) -> broadcast::Receiver<String> {
        self.sender.subscribe()
    }

    /// Caller must have committed its visible Job mutation before publishing.
    pub(crate) fn publish_committed(&self, job_id: &str) {
        let _ = self.sender.send(job_id.to_owned());
    }
}
