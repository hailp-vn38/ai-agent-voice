use std::sync::Arc;

use super::super::client::MuseVmClient;
use super::SharedMusePool;
use super::types::{HotConversation, UpstreamFailureKind};

pub(crate) struct VmOperationLease {
    pub(super) pool: SharedMusePool,
    pub(super) account_index: usize,
    pub(super) client: Arc<MuseVmClient>,
    pub(super) session_id: String,
    pub(super) matched_hot: Option<HotConversation>,
    pub(super) saved_as_hot: bool,
    pub(super) cleaned: bool,
    /// Gateway account that owns this lease's VM (None for pools built without gateway context).
    pub(super) account_key: Option<String>,
    /// Keep the Muse session alive after the operation (job-scoped continuation).
    pub(super) retain_session: bool,
}

impl VmOperationLease {
    pub async fn reconnect(
        &mut self,
    ) -> Result<
        tokio::sync::broadcast::Receiver<super::super::client::VmStreamEvent>,
        (UpstreamFailureKind, String),
    > {
        self.client = self.pool.ensure_account_client(self.account_index).await?;
        Ok(self.client.subscribe_events())
    }

    pub fn client(&self) -> &Arc<MuseVmClient> {
        &self.client
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn set_session_id(&mut self, session_id: String) {
        self.session_id = session_id;
    }

    pub fn account_key(&self) -> Option<&str> {
        self.account_key.as_deref()
    }

    /// Keeps the Muse session after this operation so a later call of the same job can continue it.
    /// Account failures (`fail_account`) keep it too, so a later attempt can resume the running turn.
    pub fn retain_session(&mut self) {
        self.retain_session = true;
    }

    /// Undoes `retain_session` when no later call will come back to this session.
    pub fn abandon_session(&mut self) {
        self.retain_session = false;
    }

    pub fn matched_hot(&self) -> Option<&HotConversation> {
        self.matched_hot.as_ref()
    }

    pub fn finish_clean(&mut self, updated_hot: Option<HotConversation>) {
        if self.cleaned {
            return;
        }
        self.cleaned = true;
        if let Some(hot) = updated_hot {
            self.saved_as_hot = true;
            self.pool.release_op_clean(self.account_index, Some(hot));
        } else {
            self.saved_as_hot = false;
            self.pool.release_op_clean(self.account_index, None);
            if !self.retain_session {
                spawn_delete_ephemeral_session(&self.client, &self.session_id);
            }
        }
    }

    pub async fn fail_account(&mut self, kind: UpstreamFailureKind) {
        if self.cleaned {
            return;
        }
        self.cleaned = true;
        self.saved_as_hot = false;
        let sid = self.session_id.trim();
        if !self.retain_session && !sid.is_empty() && self.client.is_connected() {
            let _ = self.client.delete_session(sid).await;
        }
        self.pool.fail_account(self.account_index, kind).await;
    }
}

impl Drop for VmOperationLease {
    fn drop(&mut self) {
        if !self.cleaned {
            self.cleaned = true;
            self.pool.release_op_clean(self.account_index, None);
            if !self.saved_as_hot && !self.retain_session {
                spawn_delete_ephemeral_session(&self.client, &self.session_id);
            }
        }
    }
}

fn spawn_delete_ephemeral_session(client: &Arc<MuseVmClient>, session_id: &str) {
    let sid = session_id.trim();
    if sid.is_empty() || !client.is_connected() {
        return;
    }
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        let client_clone = Arc::clone(client);
        let sid_owned = sid.to_string();
        handle.spawn(async move {
            let _ = client_clone.delete_session(&sid_owned).await;
        });
    }
}
