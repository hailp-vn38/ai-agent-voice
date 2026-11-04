use super::*;

/// The application-owned handles that keep one connection countable and closable.
///
/// Holding `registration` is what keeps this connection in the drain registry: it is registered
/// before the upgrade and released when the socket task ends, so the registry's count is a fact
/// about live connections rather than a guess.  The two signals are separate decisions — `stopping`
/// is the process finishing, `drain_close` is the deadline reaching *this* connection — and a
/// controlled close is a request the session performs itself, not a force.
pub(super) struct SessionControl {
    pub(super) stopping: CancellationToken,
    pub(super) drain_close: CancellationToken,
    pub(super) registration: crate::lifecycle::DrainRegistration,
}

/// Why the application ended a Voice Session from outside its own protocol flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CloseReason {
    /// The process is finishing outright.
    Stopping,
    /// The shutdown drain deadline reached this session: Controlled Close.
    DrainDeadline,
    /// A newer connection of the same Device, or a revoked Web Assistant credential, replaces it.
    Replaced,
    /// A security mutation revoked a snapshot the session pinned (System MCP grants, External MCP
    /// tools, speaker policy).
    Revoked,
}

/// Every external signal that can end one Voice Session, observed in one place and listed once,
/// in priority order: shutdown signals win when several fire together.
pub(super) struct SessionCloseSet {
    signals: Vec<(CloseReason, CancellationToken)>,
    replaced: CancellationToken,
}

impl SessionCloseSet {
    /// `revocations` are the security snapshots this session pinned (System MCP grants, External
    /// MCP tools, speaker policy); `None` for a capability the session does not hold.
    pub(super) fn new(
        stopping: CancellationToken,
        drain_deadline: CancellationToken,
        assistant_revoked: Option<CancellationToken>,
        revocations: impl IntoIterator<Item = Option<CancellationToken>>,
    ) -> Self {
        let replaced = CancellationToken::new();
        let mut signals = vec![
            (CloseReason::Stopping, stopping),
            (CloseReason::DrainDeadline, drain_deadline),
            (CloseReason::Replaced, replaced.clone()),
        ];
        signals.extend(assistant_revoked.map(|token| (CloseReason::Replaced, token)));
        signals.extend(
            revocations
                .into_iter()
                .flatten()
                .map(|token| (CloseReason::Revoked, token)),
        );
        Self { signals, replaced }
    }

    /// The token a newer connection of the same Device cancels to take this session's place.
    pub(super) fn replacement(&self) -> CancellationToken {
        self.replaced.clone()
    }

    /// Completes with the first signal that fired; on a tie the earlier entry wins.
    pub(super) async fn cancelled(&self) -> CloseReason {
        let waits = self.signals.iter().map(|(reason, token)| {
            Box::pin(async move {
                token.cancelled().await;
                *reason
            })
        });
        futures_util::future::select_all(waits).await.0
    }

    /// The same signals for SessionActor, which stops new work as soon as any fires.
    pub(super) fn tokens(&self) -> impl Iterator<Item = CancellationToken> + '_ {
        self.signals.iter().map(|(_, token)| token.clone())
    }
}
