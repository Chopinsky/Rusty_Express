#![allow(dead_code)]

use channel;
use chrono::{DateTime, Utc};
use std::sync::{Mutex, Once, RwLock, ONCE_INIT};
use std::thread;
use std::time::Duration;

lazy_static! {
    static ref TEMP_STORE: Mutex<Vec<LogInfo>> = Mutex::new(Vec::new());
    static ref CONFIG: RwLock<LoggerConfig> = RwLock::new(LoggerConfig::initialize());
}

static ONCE: Once = ONCE_INIT;
static mut SENDER: Option<channel::Sender<LogInfo>> = None;
static mut REFRESH_HANDLER: Option<thread::JoinHandle<()>> = None;

pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Critical,
}

struct LogInfo {
    message: String,
    level: LogLevel,
    time: DateTime<Utc>,
}

struct LoggerConfig {
    refresh_period: Duration,
    store_folder: String,
}

impl LoggerConfig {
    fn initialize() -> Self {
        LoggerConfig {
            refresh_period: Duration::from_secs(1800),
            store_folder: String::new(),
        }
    }

    fn get_refresh_period(&self) -> Duration {
        self.refresh_period.to_owned()
    }

    fn set_refresh_period(&mut self, period: Duration) {
        self.refresh_period = period;
    }
}

pub(crate) fn initialize() {
    ONCE.call_once(|| {
        let (tx, rx): (channel::Sender<LogInfo>, channel::Receiver<LogInfo>) = channel::unbounded();

        unsafe {
            SENDER = Some(tx);

            if let Ok(config) = CONFIG.read() {
                if config.store_folder.is_empty() {
                    return;
                } else {
                    let refresh = config.refresh_period.as_secs();
                    println!(
                        "The logger has started, it will refresh log to folder {} every {} seconds",
                        config.store_folder, refresh
                    );

                    start_refresh(config.refresh_period.clone());
                }
            }
        }
    });
}

fn start_refresh(period: Duration) {
    unsafe {
        REFRESH_HANDLER = Some(thread::spawn(move || loop {
            thread::sleep(period);
            write_to_file();
        }));
    }
}

fn stop_refresh() {
    unsafe {
        if let Some(handler) = REFRESH_HANDLER.take() {
            handler.join().unwrap_or_else(|err| {
                eprintln!(
                    "Failed to stop the log refresh service, error code: {:?}...",
                    err
                );
            });
        }
    }
}

fn reset_refresh(period: Option<Duration>) {
    thread::spawn(move || {
        stop_refresh();

        let new_period = if let Ok(mut config) = CONFIG.write() {
            if let Some(p) = period {
                config.set_refresh_period(p);
            }

            config.get_refresh_period()
        } else {
            Duration::from_secs(1800)
        };

        start_refresh(new_period);
    });
}

fn write_to_file() {}
