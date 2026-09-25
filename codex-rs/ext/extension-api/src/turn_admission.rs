//! Lets Core turn-input submissions participate in a host's shutdown drain.

/// A host-provided gate checked before Core starts a turn-input submission.
///
/// Implementations return a permit for work admitted before shutdown and
/// Core retains it through submission. `None` skips the start without consuming
/// pending input. Steering an existing turn does not acquire a new permit.
/// Memory-only mailbox wakeups and parent-delegated subagent input bypass this
/// gate so delegated work can finish before exit. Automatic starts remain gated.
pub trait TurnStartAdmission: std::fmt::Debug + Send + Sync {
    fn admit_turn_start(&self) -> Option<Box<dyn Send>>;
}
