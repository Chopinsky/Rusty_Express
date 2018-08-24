#![allow(dead_code)]

use channel;
use chrono::{DateTime, Utc};
use std::env;
use std::path::PathBuf;
use std::sync::{atomic::{AtomicBool, Ordering}, Mutex, Once, RwLock, ONCE_INIT};
use std::thread;
use std::time::Duration;

lazy_static! {
    static ref TEMP_STORE: Mutex<Vec<LogInfo>> = Mutex::new(Vec::new());
    static ref CONFIG: RwLock<LoggerConfig> = RwLock::new(LoggerConfig::initialize());
}

static ONCE: Once = ONCE_INIT;
static mut SENDER: Option<channel::Sender<LogInfo>> = None;
static mut REFRESH_HANDLER: Option<thread::JoinHandle<()>> = None;
static mut DUMPING_RUNNING: AtomicBool = AtomicBool::new(false);

pub enum InfoLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Critical,
}

struct LogInfo {
    message: String,
    level: InfoLevel,
    time: DateTime<Utc>,
}

struct LoggerConfig {
    refresh_period: Duration,
    log_folder_path: Option<PathBuf>,
    meta_info_provider: Option<fn() -> String>,
    rx_handler: Option<thread::JoinHandle<()>>
}

impl LoggerConfig {
    fn initialize() -> Self {
        LoggerConfig {
            refresh_period: Duration::from_secs(1800),
            log_folder_path: None,
            meta_info_provider: None,
            rx_handler: None,
        }
    }

    #[inline]
    fn get_refresh_period(&self) -> Duration {
        self.refresh_period.to_owned()
    }

    #[inline]
    fn set_refresh_period(&mut self, period: Duration) {
        self.refresh_period = period;
    }

    pub fn set_log_folder_path(&mut self, path: &str) {
        let mut path_buff = PathBuf::new();

        let location: Option<PathBuf> =
            if path.is_empty() {
                match env::var_os("LOG_FOLDER_PATH") {
                    Some(p) => {
                        path_buff.push(p);
                        Some(path_buff)
                    },
                    None => None,
                }
            } else {
                path_buff.push(path);
                Some(path_buff)
            };

        if let Some(loc) = location {
            if loc.as_path().is_dir() {
                self.log_folder_path = Some(loc);
            }
        }
    }

    #[inline]
    pub fn get_log_folder_path(&self) -> Option<PathBuf> {
        self.log_folder_path.clone()
    }
}

pub fn log(message: &str, level: InfoLevel) -> Result<(), String> {
    if let Ok(mut store) = TEMP_STORE.lock() {
        let info = LogInfo {
            message: message.to_owned(),
            level,
            time: Utc::now(),
        };

        store.push(info);
        return Ok(());
    }

    Err(String::from("Failed to add the info to the backlog"))
}

pub(crate) fn logger_run(
    period: Option<u64>,
    log_folder_path: Option<&str>,
    meta_info_provider: Option<fn() -> String>)
{
    if let Ok(mut config) = CONFIG.write() {
        if let Some(time) = period {
            if time != 1800 {
                config.refresh_period = Duration::from_secs(time);
            }
        }

        if let Some(path) = log_folder_path {
            config.set_log_folder_path(path);
        }
        
        config.meta_info_provider = meta_info_provider;
    }

    initialize();
}

pub(crate) fn logger_cleanup() {
    stop_refresh();

    if let Ok(mut config) = CONFIG.write() {
        if let Some(rx) = config.rx_handler.take() {
            rx.join().unwrap_or_else(|err| {
                eprintln!("Encountered error while closing the logger: {:?}", err);
            });
        }
    }
}

fn initialize() {
    ONCE.call_once(|| {
        let (tx, rx): (channel::Sender<LogInfo>, channel::Receiver<LogInfo>) = channel::unbounded();

        unsafe { SENDER = Some(tx); }

        if let Ok(mut config) = CONFIG.write() {
            if let Some(ref path) = config.log_folder_path {
                let refresh = config.refresh_period.as_secs();

                println!(
                    "The logger has started, it will refresh log to folder {:?} every {} seconds",
                    path.to_str().unwrap(), refresh
                );

                start_refresh(config.refresh_period.clone());
            }

            config.rx_handler = Some(thread::spawn(move || {
                for info in rx {
                    if let Ok(mut store) = TEMP_STORE.lock() {
                        store.push(info);
                    }
                }
            }));
        }
    });
}

fn start_refresh(period: Duration) {
    unsafe {
        if REFRESH_HANDLER.is_some() {
            stop_refresh();
        }

        REFRESH_HANDLER = Some(thread::spawn(move ||
            loop {
                thread::sleep(period);
                thread::spawn(|| {
                    dump_to_file();
                });
            }
        ));
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

        let new_period =
            if let Ok(mut config) = CONFIG.write() {
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

fn dump_to_file() {
    unsafe {
        if DUMPING_RUNNING.load(Ordering::Relaxed) {
            //TODO: write only the meta info + why we skip this dump
        }

        if let Ok(config) = CONFIG.read() {
            *DUMPING_RUNNING.get_mut() = true;

            //TODO: writing to file
            if let Ok(mut store) = TEMP_STORE.lock() {
                let mut content: String;
                while let Some(info) = store.pop() {
                    content = format!("{}", info.message);
                }
            }

            *DUMPING_RUNNING.get_mut() = false;
        }
    }
}
