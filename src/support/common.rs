use std::time::Duration;
use chrono;

pub fn chrono_to_std(duration: chrono::Duration) -> Option<Duration> {
    Some(Duration::from_millis(duration.num_milliseconds() as u64))
}

pub fn std_to_chrono(duration: Duration) -> Option<chrono::Duration> {
    if let Ok(period) = chrono::Duration::from_std(duration) {
        Some(period)
    } else {
        None
    }
}