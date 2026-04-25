#[cfg(feature = "sleep-inhibitor")]
pub(crate) use codex_utils_sleep_inhibitor::SleepInhibitor;

#[cfg(not(feature = "sleep-inhibitor"))]
#[derive(Debug)]
pub(crate) struct SleepInhibitor {
    turn_running: bool,
}

#[cfg(not(feature = "sleep-inhibitor"))]
impl SleepInhibitor {
    pub(crate) fn new(_enabled: bool) -> Self {
        Self {
            turn_running: false,
        }
    }

    pub(crate) fn set_turn_running(&mut self, turn_running: bool) {
        self.turn_running = turn_running;
    }

    #[cfg(test)]
    pub(crate) fn is_turn_running(&self) -> bool {
        self.turn_running
    }
}
