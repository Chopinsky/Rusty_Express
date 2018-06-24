use std::env;
use std::sync::{Once, ONCE_INIT};
use chrono::prelude::{DateTime, Utc};

static ONCE: Once = ONCE_INIT;
static mut DEBUG_LEVEL: u8 = 0;

pub fn initialize() {
    ONCE.call_once(|| {
        if let Ok(debug_mode) = env::var("DEBUG_LEVEL") {
            match &debug_mode[..] {
                "1" => set_debug_level(1),
                "2" => set_debug_level(2),
                "3" => set_debug_level(3),
                _ => set_debug_level(0),
            }
        }
    });
}

pub fn print(info: &str, level: u8) {
    if !in_debug_mode() { return; }
    if info.is_empty() { return; }
    if !print_level_allowed(level) { return; }

    let now: DateTime<Utc> = Utc::now();
    let level_label = match level {
        0 => String::from("info"),
        1 => String::from("warning"),
        _ => String::from(format!("error [{}]", level)),
    };

    println!("{}: {} at {}", level_label, now.format("%Y-%m-%d %H:%M:%S GMT").to_string(), info);
}

#[inline]
fn in_debug_mode() -> bool {
    unsafe { DEBUG_LEVEL > 0 }
}

#[inline]
fn print_level_allowed(level: u8) -> bool {
    unsafe { DEBUG_LEVEL >= level }
}

fn set_debug_level(debug: u8) {
    unsafe {
        DEBUG_LEVEL = debug;

        if in_debug_mode() {
            println!("\n\tNow in debug mode...\n");
        }
    }
}
