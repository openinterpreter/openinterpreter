use super::*;
use pretty_assertions::assert_eq;

#[test]
fn guardian_rejection_circuit_breaker_interrupts_after_three_consecutive_denials() {
    let mut circuit_breaker = GuardianRejectionCircuitBreaker::default();
    assert_eq!(
        circuit_breaker.record_denial("turn-1", GuardianRejectionCircuitBreakerPolicy::Standard),
        GuardianRejectionCircuitBreakerAction::Continue
    );
    assert_eq!(
        circuit_breaker.record_denial("turn-1", GuardianRejectionCircuitBreakerPolicy::Standard),
        GuardianRejectionCircuitBreakerAction::Continue
    );
    assert_eq!(
        circuit_breaker.record_denial("turn-1", GuardianRejectionCircuitBreakerPolicy::Standard),
        GuardianRejectionCircuitBreakerAction::InterruptTurn {
            consecutive_denials: 3,
            recent_denials: 3,
        }
    );
    assert_eq!(
        circuit_breaker.record_denial("turn-1", GuardianRejectionCircuitBreakerPolicy::Standard),
        GuardianRejectionCircuitBreakerAction::Continue
    );
}

#[test]
fn guardian_rejection_circuit_breaker_interrupts_cyber_models_after_one_denial() {
    let mut circuit_breaker = GuardianRejectionCircuitBreaker::default();
    assert_eq!(
        circuit_breaker.record_denial("turn-1", GuardianRejectionCircuitBreakerPolicy::CyberModel),
        GuardianRejectionCircuitBreakerAction::InterruptTurn {
            consecutive_denials: 1,
            recent_denials: 1,
        }
    );
    assert_eq!(
        circuit_breaker.record_denial("turn-1", GuardianRejectionCircuitBreakerPolicy::CyberModel),
        GuardianRejectionCircuitBreakerAction::Continue
    );
}

#[test]
fn guardian_rejection_circuit_breaker_resets_consecutive_denials_on_non_denial() {
    let mut circuit_breaker = GuardianRejectionCircuitBreaker::default();
    assert_eq!(
        circuit_breaker.record_denial("turn-1", GuardianRejectionCircuitBreakerPolicy::Standard),
        GuardianRejectionCircuitBreakerAction::Continue
    );
    circuit_breaker.record_non_denial("turn-1");
    assert_eq!(
        circuit_breaker.record_denial("turn-1", GuardianRejectionCircuitBreakerPolicy::Standard),
        GuardianRejectionCircuitBreakerAction::Continue
    );
    assert_eq!(
        circuit_breaker.record_denial("turn-1", GuardianRejectionCircuitBreakerPolicy::Standard),
        GuardianRejectionCircuitBreakerAction::Continue
    );
    assert_eq!(
        circuit_breaker.record_denial("turn-1", GuardianRejectionCircuitBreakerPolicy::Standard),
        GuardianRejectionCircuitBreakerAction::InterruptTurn {
            consecutive_denials: 3,
            recent_denials: 4,
        }
    );
}

#[test]
fn auto_review_rejection_circuit_breaker_interrupts_after_ten_recent_denials() {
    let mut circuit_breaker = GuardianRejectionCircuitBreaker::default();
    for _ in 0..9 {
        assert_eq!(
            circuit_breaker
                .record_denial("turn-1", GuardianRejectionCircuitBreakerPolicy::Standard),
            GuardianRejectionCircuitBreakerAction::Continue
        );
        circuit_breaker.record_non_denial("turn-1");
    }
    assert_eq!(
        circuit_breaker.record_denial("turn-1", GuardianRejectionCircuitBreakerPolicy::Standard),
        GuardianRejectionCircuitBreakerAction::InterruptTurn {
            consecutive_denials: 1,
            recent_denials: 10,
        }
    );
}

#[test]
fn auto_review_rejection_circuit_breaker_forgets_denials_outside_recent_review_window() {
    let mut circuit_breaker = GuardianRejectionCircuitBreaker::default();
    for _ in 0..9 {
        assert_eq!(
            circuit_breaker
                .record_denial("turn-1", GuardianRejectionCircuitBreakerPolicy::Standard),
            GuardianRejectionCircuitBreakerAction::Continue
        );
        circuit_breaker.record_non_denial("turn-1");
    }
    for _ in 0..(AUTO_REVIEW_DENIAL_WINDOW_SIZE - 18) {
        circuit_breaker.record_non_denial("turn-1");
    }
    assert_eq!(
        circuit_breaker.record_denial("turn-1", GuardianRejectionCircuitBreakerPolicy::Standard),
        GuardianRejectionCircuitBreakerAction::Continue
    );
}
