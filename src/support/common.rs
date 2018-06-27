use std::collections::HashMap;
use std::time::Duration;
use chrono;

use super::debug;
use std::io::{BufWriter, Write};
use std::net::TcpStream;

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
    fn add(&mut self, field: &str, value: T, allow_replace: bool) -> Option<T>;
}

impl<T> MapUpdates<T> for HashMap<String, T> {
    fn add(&mut self, field: &str, value: T, allow_replace: bool) -> Option<T> {
        if field.is_empty() { return None; }

        let f = field.to_lowercase();
        if allow_replace {
            //new field, insert into the map
            self.insert(f, value)
        } else {
            //existing field, replace existing value or append depending on the parameter
            self.entry(f).or_insert(value);
            None
        }
    }
}

pub fn write_to_buff(buffer: &mut BufWriter<&TcpStream>, content: &[u8]) {
    if let Err(err) = buffer.write(content) {
        debug::print(
            &format!("An error has taken place when writing the response header to the stream: {}", err),
            1);
    }
}

pub fn flush_buffer(buffer: &mut BufWriter<&TcpStream>) -> u8 {
    if let Err(err) = buffer.flush() {
        debug::print(
            &format!("An error has taken place when flushing the response to the stream: {}", err)[..],
            1);

        return 1
    }

    0
}