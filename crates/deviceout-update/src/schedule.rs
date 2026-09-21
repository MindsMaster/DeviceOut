use crate::State;

pub const CHECK_SUCCESS_INTERVAL_SECS: i64 = 6 * 60 * 60;
pub const CHECK_FAILURE_INTERVAL_SECS: i64 = 60 * 60;

pub fn within(last: Option<i64>, now: i64, window_secs: i64) -> bool {
    match last {
        Some(last) => (0..window_secs).contains(&(now - last)),
        None => false,
    }
}

pub fn should_check(state: &State, now: i64, force: bool) -> bool {
    if force {
        return true;
    }
    if within(state.last_check, now, CHECK_SUCCESS_INTERVAL_SECS) {
        return false;
    }
    !within(state.last_attempt, now, CHECK_FAILURE_INTERVAL_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(last_check: Option<i64>, last_attempt: Option<i64>) -> State {
        State {
            last_check,
            last_attempt,
            ..State::default()
        }
    }

    #[test]
    fn first_run_checks() {
        assert!(should_check(&state(None, None), 1_000, false));
    }

    #[test]
    fn a_success_holds_for_one_interval() {
        let s = state(Some(1_000), Some(1_000));
        assert!(!should_check(&s, 1_000 + CHECK_SUCCESS_INTERVAL_SECS - 1, false));
        assert!(should_check(&s, 1_000 + CHECK_SUCCESS_INTERVAL_SECS, false));
    }

    #[test]
    fn failure_holds_for_an_hour_only() {
        let s = state(None, Some(5_000));
        assert!(!should_check(&s, 5_000 + CHECK_FAILURE_INTERVAL_SECS - 1, false));
        assert!(should_check(&s, 5_000 + CHECK_FAILURE_INTERVAL_SECS, false));
    }

    #[test]
    fn old_success_with_recent_failure_backs_off() {
        let s = state(Some(0), Some(90_000));
        assert!(!should_check(&s, 90_000 + 60, false));
    }

    #[test]
    fn clock_moving_backwards_does_not_freeze_checks() {
        let s = state(Some(1_000_000), Some(1_000_000));
        assert!(should_check(&s, 500, false));
        assert!(!within(Some(1_000_000), 500, 60));
    }

    #[test]
    fn force_ignores_every_window() {
        let s = state(Some(1_000), Some(1_000));
        assert!(should_check(&s, 1_001, true));
    }
}
