//! Standalone ASR diagnostics over an already-materialized runtime provider.

use std::{
    sync::{Arc, mpsc},
    thread,
};

use tokio::sync::mpsc as session_mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    audio::PcmF32Mono,
    providers::AsrProvider,
    services::provider_diagnostic::{
        ProviderDiagnosticOperation, ProviderDiagnosticOperationError,
    },
};

use super::AsrCommand;

/// One bounded, standalone ASR request using an already-materialized provider runtime.
pub struct AsrDiagnosticOperation {
    provider: Arc<dyn AsrProvider>,
    pcm: PcmF32Mono,
    commands: Option<mpsc::Sender<AsrCommand>>,
    events: Option<session_mpsc::Receiver<AsrDiagnosticEvent>>,
    terminal: bool,
}

enum AsrDiagnosticEvent {
    Final(String),
    Failed,
    Cancelled,
}

impl AsrDiagnosticOperation {
    pub(super) fn new(provider: Arc<dyn AsrProvider>, pcm: PcmF32Mono) -> Self {
        Self {
            provider,
            pcm,
            commands: None,
            events: None,
            terminal: false,
        }
    }
}

#[async_trait::async_trait]
impl ProviderDiagnosticOperation for AsrDiagnosticOperation {
    type Output = String;

    async fn execute(
        &mut self,
        cancellation: CancellationToken,
    ) -> Result<Self::Output, ProviderDiagnosticOperationError> {
        let (commands_tx, commands_rx) = mpsc::channel();
        let (events_tx, events_rx) = session_mpsc::channel(1);
        let provider = Arc::clone(&self.provider);
        let pcm = self.pcm.clone();
        thread::spawn(move || run_diagnostic(provider, pcm, commands_rx, events_tx));
        self.commands = Some(commands_tx);
        self.events = Some(events_rx);
        let event = tokio::select! {
            _ = cancellation.cancelled() => return Err(ProviderDiagnosticOperationError::Failed),
            event = self.events.as_mut().expect("ASR diagnostic events initialized").recv() => event,
        };
        match event {
            Some(AsrDiagnosticEvent::Final(text)) => {
                self.terminal = true;
                Ok(text)
            }
            Some(AsrDiagnosticEvent::Failed) | Some(AsrDiagnosticEvent::Cancelled) | None => {
                self.terminal = true;
                Err(ProviderDiagnosticOperationError::Failed)
            }
        }
    }

    fn cancel_exact(&mut self) {
        if let Some(commands) = &self.commands {
            let _ = commands.send(AsrCommand::Cancel);
        }
    }

    async fn await_terminal_acknowledgement(&mut self) -> bool {
        if self.terminal {
            return true;
        }
        let Some(events) = &mut self.events else {
            return false;
        };
        match events.recv().await {
            Some(AsrDiagnosticEvent::Final(_))
            | Some(AsrDiagnosticEvent::Failed)
            | Some(AsrDiagnosticEvent::Cancelled) => {
                self.terminal = true;
                true
            }
            None => false,
        }
    }

    fn quarantine_exact(&mut self) {
        // The service retains the exact runtime-capacity permit until process teardown. This
        // operation owns no reusable slot beyond that permit.
    }
}

fn run_diagnostic(
    provider: Arc<dyn AsrProvider>,
    pcm: PcmF32Mono,
    commands: mpsc::Receiver<AsrCommand>,
    events: session_mpsc::Sender<AsrDiagnosticEvent>,
) {
    let Ok(mut session) = provider.open() else {
        let _ = events.blocking_send(AsrDiagnosticEvent::Failed);
        return;
    };
    let terminal = if matches!(commands.try_recv(), Ok(AsrCommand::Cancel)) {
        session.cancel();
        AsrDiagnosticEvent::Cancelled
    } else if session.push_pcm(&pcm).is_err() {
        AsrDiagnosticEvent::Failed
    } else if matches!(commands.try_recv(), Ok(AsrCommand::Cancel)) {
        session.cancel();
        AsrDiagnosticEvent::Cancelled
    } else {
        match session.finish() {
            Ok(result) => AsrDiagnosticEvent::Final(result.text().to_owned()),
            Err(_) => AsrDiagnosticEvent::Failed,
        }
    };
    // Returning retained state/reset or native destruction must precede terminal ack.
    drop(session);
    let _ = events.blocking_send(terminal);
}
