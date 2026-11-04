#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionPhase {
    Ready,
    Listening,
    Processing,
    Speaking,
    Closed,
}
