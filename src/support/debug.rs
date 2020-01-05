use std::env;

use crate::chrono::prelude::{DateTime, Utc};
use crate::parking_lot::Once;

static ONCE: Once = Once::new();
static mut DEBUG_LEVEL: InfoLevel = InfoLevel::Silent;

#[derive(PartialEq)]
pub enum InfoLevel {
    Silent,
    Info,
    Warning,
    Error,
}

pub fn initialize() {
    ONCE.call_once(|| {
        if let Ok(debug_mode) = env::var("DEBUG_LEVEL") {
            match &debug_mode[..] {
                "1" => set_debug_level(InfoLevel::Info),
                "2" => set_debug_level(InfoLevel::Warning),
                "3" => set_debug_level(InfoLevel::Error),
                _ => set_debug_level(InfoLevel::Silent),
            }
        }
    });
}

pub fn print(info: &str, level: InfoLevel) {
    if !in_debug_mode() {
        return;
    }
    if info.is_empty() {
        return;
    }
    if !print_level_allowed(&level) {
        return;
    }

    let now: DateTime<Utc> = Utc::now();
    let level_label = match level {
        InfoLevel::Info => String::from("Info"),
        InfoLevel::Warning => String::from("Warning"),
        InfoLevel::Error => String::from("Error"),
        InfoLevel::Silent => return,
    };

    eprintln!("\r\n======================");
    eprintln!(
        "[{}] at {}:\r\n {}",
        level_label,
        now.format("%Y-%m-%d %H:%M:%S GMT").to_string(),
        info
    );
}

#[inline]
fn in_debug_mode() -> bool {
    unsafe {
        match DEBUG_LEVEL {
            InfoLevel::Silent => false,
            _ => true,
        }
    }
}

#[inline]
fn print_level_allowed(level: &InfoLevel) -> bool {
    unsafe {
        let raw_level = cast_info_level(level);
        match DEBUG_LEVEL {
            InfoLevel::Silent => false,
            InfoLevel::Error => raw_level > 2,
            InfoLevel::Warning => raw_level > 1,
            _ => true,
        }
    }
}

fn cast_info_level(level: &InfoLevel) -> u8 {
    match level {
        InfoLevel::Silent => 0,
        InfoLevel::Info => 1,
        InfoLevel::Warning => 2,
        InfoLevel::Error => 3,
    }
}

fn set_debug_level(debug: InfoLevel) {
    unsafe {
        DEBUG_LEVEL = debug;

        if in_debug_mode() {
            println!("\n\tNow in debug mode...\n");
        }
    }
}
