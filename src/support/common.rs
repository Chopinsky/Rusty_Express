use std::collections::HashMap;
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

pub trait MapUpdates<T> {
    fn add(&mut self, field: &str, value: T, allow_override: bool) -> Option<T>;
}

impl<T> MapUpdates<T> for HashMap<String, T> {
    fn add(&mut self, field: &str, value: T, allow_override: bool) -> Option<T> {
        if field.is_empty() { return None; }

        let f = field.to_lowercase();
        if allow_override {
            //new field, insert into the map
            self.insert(f, value)
        } else {
            //existing field, replace existing value or append depending on the parameter
            self.entry(f).or_insert(value);
            None
        }
    }
}