use std::cell::Cell;

/// Process-local privacy counters (also mirrored into SQLite by the UI layer).
#[derive(Debug, Default)]
pub struct PrivacyStats {
    blocked_requests: Cell<u64>,
    allowed_requests: Cell<u64>,
    cosmetic_injections: Cell<u64>,
}

impl PrivacyStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_block(&self) {
        self.blocked_requests.set(self.blocked_requests.get() + 1);
    }

    pub fn record_allow(&self) {
        self.allowed_requests.set(self.allowed_requests.get() + 1);
    }

    pub fn record_cosmetic(&self) {
        self.cosmetic_injections.set(self.cosmetic_injections.get() + 1);
    }

    pub fn blocked(&self) -> u64 {
        self.blocked_requests.get()
    }

    pub fn allowed(&self) -> u64 {
        self.allowed_requests.get()
    }

    pub fn cosmetic(&self) -> u64 {
        self.cosmetic_injections.get()
    }

    pub fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            blocked_requests: self.blocked(),
            allowed_requests: self.allowed(),
            cosmetic_injections: self.cosmetic(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StatsSnapshot {
    pub blocked_requests: u64,
    pub allowed_requests: u64,
    pub cosmetic_injections: u64,
}

impl StatsSnapshot {
    pub fn summary_line(&self) -> String {
        format!("Blocked {} trackers/ads · {} cosmetic cleanups", self.blocked_requests, self.cosmetic_injections)
    }
}
