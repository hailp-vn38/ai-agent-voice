/// Speech provenance stays with the actor and never selects a provider or Device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpeechOrigin {
    Llm,
    ToolDirect,
    ClientDirect,
    AdminDirect,
    Reminder {
        reminder_id: String,
        attempt_id: String,
    },
}

#[derive(Clone, Debug)]
pub struct DirectSpeechRequest {
    pub request_id: String,
    pub text: String,
    pub origin: SpeechOrigin,
}
