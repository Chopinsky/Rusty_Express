use std::time::Duration;
use chrono;

pub fn to_std_duration(duration: chrono::Duration) -> Option<Duration> {
    Some(Duration::from_millis(duration.num_milliseconds() as u64))
}

pub fn from_std_duration(duration: Duration) -> Option<chrono::Duration> {
    if let Ok(period) = chrono::Duration::from_std(duration) {
        Some(period)
    } else {
        None
    }
}