use std::fmt::Display;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Backoff {
    date: DateTime<Utc>,
    time: u64,
    // Whether a "down" alert has been sent for this outage
    #[serde(default)]
    notified: bool,
}

impl Backoff {
    pub fn with_time(time: u64) -> Self {
        Backoff {
            date: Utc::now(),
            time,
            notified: false,
        }
    }

    pub fn get_date(&self) -> DateTime<Utc> {
        self.date
    }

    pub fn is_notified(&self) -> bool {
        self.notified
    }

    pub fn mark_notified(&mut self) {
        self.notified = true;
    }

    pub fn increment_by_factor(&mut self) {
        self.date = self.date + Duration::minutes(self.time as i64) + Duration::seconds(2);
        self.time += 1;
    }
}

impl Display for Backoff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Backoff date: {}, time: {}", self.date, self.time)
    }
}
