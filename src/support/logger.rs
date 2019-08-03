#![allow(dead_code)]

use std::env;
use std::fmt;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::mem;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;

use crate::channel::{self, SendError};
use crate::chrono::{DateTime, Utc};
use crate::core::syncstore::StaticStore;
use crate::parking_lot::{Once, ONCE_INIT};
use crate::support::{common::cpu_relax, debug};

//lazy_static! {
//    static ref TEMP_STORE: Mutex<Vec<LogInfo>> = Mutex::new(Vec::new());
//    static ref CONFIG: RwLock<LoggerConfig> = RwLock::new(LoggerConfig::initialize(""));
//}

const DEFAULT_LOCATION: &str = "./logs";

static ONCE: Once = ONCE_INIT;
static DUMP_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

static mut CHAN: Option<(channel::Sender<LogMessage>, channel::Receiver<LogMessage>)> = None;
static mut CONFIG: StaticStore<LoggerConfig> = StaticStore::init();
static mut REFRESH_HANDLER: Option<thread::JoinHandle<()>> = None;

#[derive(Debug)]
pub enum InfoLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Critical,
}

enum LogMessage {
    Info(LogInfo),
    Shutdown,
    Dump,
}

impl fmt::Display for InfoLevel {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

pub struct LogInfo {
    message: String,
    client: Option<SocketAddr>,
    level: InfoLevel,
    time: DateTime<Utc>,
}

impl fmt::Display for LogInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.client {
            Some(addr) => write!(
                f,
                "[{}] @ {} (from client {}): {}",
                self.level,
                self.time.to_string(),
                addr.to_string(),
                self.message
            ),
            None => write!(
                f,
                "[{}] @ {}: {}",
                self.level,
                self.time.to_string(),
                self.message
            ),
        }
    }
}

struct LoggerConfig {
    id: String,
    refresh_period: Duration,
    log_folder_path: Option<PathBuf>,
    meta_info_provider: Option<Box<dyn Fn(bool) -> String>>,
    rx_handler: Option<thread::JoinHandle<()>>,
    write_lock: AtomicBool,
}

impl LoggerConfig {
    fn initialize(id: &str) -> Self {
        LoggerConfig {
            id: id.to_owned(),
            refresh_period: Duration::from_secs(1800),
            log_folder_path: None,
            meta_info_provider: None,
            rx_handler: None,
            write_lock: AtomicBool::new(false),
        }
    }

    #[inline]
    fn set_id(&mut self, id: &str) {
        //TODO: internal lock to check write privilege
        self.id = id.to_owned();
    }

    #[inline]
    fn get_id(&self) -> String {
        self.id.clone()
    }

    #[inline]
    fn get_refresh_period(&self) -> Duration {
        self.refresh_period.to_owned()
    }

    #[inline]
    fn set_refresh_period(&mut self, period: Duration) {
        //TODO: internal lock to check write privilege
        self.refresh_period = period;
    }

    pub fn set_log_folder_path(&mut self, path: &str) {
        //TODO: internal lock to check write privilege

        let mut path_buff = PathBuf::new();

        let location: Option<PathBuf> = if path.is_empty() {
            match env::var_os("LOG_FOLDER_PATH") {
                Some(p) => {
                    path_buff.push(p);
                    Some(path_buff)
                }
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

pub trait LogWriter {
    fn dump(&self, log_store: &[LogInfo]) -> Result<(), usize>;
}

struct DefaultLogWriter;

impl DefaultLogWriter {
    fn get_log_file(config: &LoggerConfig) -> Result<File, String> {
        match config.log_folder_path {
            Some(ref location) if location.is_dir() => create_dump_file(config.get_id(), location),
            _ => create_dump_file(config.get_id(), &PathBuf::from(DEFAULT_LOCATION)),
        }
    }
}

impl LogWriter for DefaultLogWriter {
    fn dump(&self, log_store: &[LogInfo]) -> Result<(), usize> {
        let config = unsafe {
            match CONFIG.as_ref() {
                Ok(c) => c,
                Err(e) => return Err(0),
            }
        };

        if let Ok(mut file) = DefaultLogWriter::get_log_file(&config) {
            // Now start a new dump
            if let Some(meta_func) = config.meta_info_provider.as_ref() {
                write_to_file(&mut file, &meta_func(true));
            }

            let mut content: String = String::new();

            for (count, info) in log_store.iter().enumerate() {
                content.push_str(&format_content(&info.level, &info.message, info.time));

                if count % 10 == 0 {
                    write_to_file(&mut file, &content);
                    content.clear();
                }
            }

            // write the remainder of the content
            if !content.is_empty() {
                write_to_file(&mut file, &content);
            }

            return Ok(());
        }

        Err(0)
    }
}

pub fn log(message: &str, level: InfoLevel, client: Option<SocketAddr>) -> Result<(), String> {
    let info = LogInfo {
        message: message.to_owned(),
        client,
        level,
        time: Utc::now(),
    };

    if let Some(chan) = unsafe { CHAN.as_ref() } {
        return chan.0.send(LogMessage::Info(info)).map_err(|err| {
            format!("Failed to log the message: {:?}", err)
        });
    }

    Err(String::from("The logging service is not running..."))
}

pub(crate) fn start<T>(
    writer: T,
    period: Option<u64>,
    folder_path: Option<&str>,
    provider: Option<Box<dyn Fn(bool) -> String>>,
) where
    T: LogWriter + Send + Sync + 'static,
{
    let mut config = LoggerConfig::initialize("");

    if let Some(time) = period {
        if time != 1800 {
            config.refresh_period = Duration::from_secs(time);
        }
    }

    if let Some(path) = folder_path {
        config.set_log_folder_path(path);
    }

    config.meta_info_provider = provider;

    ONCE.call_once(|| {
        let (tx, rx) = channel::bounded(64);
        unsafe { CHAN.replace((tx, rx)); }
    });

    if let Some(ref path) = config.log_folder_path {
        let refresh = config.refresh_period.as_secs();

        println!(
            "The logger has started, it will refresh log to folder {:?} every {} seconds",
            path.to_str().unwrap(),
            refresh
        );

        start_refresh(config.refresh_period);
    }

    config.rx_handler.replace(thread::spawn(move ||
        run(Box::new(DefaultLogWriter))
    ));
}

pub(crate) fn shutdown() {
    stop_refresh();

    let config =
        match unsafe { CONFIG.as_mut() } {
            Ok(c) => c,
            Err(_) => return,
        };

    if let Some(rx) = config.rx_handler.take() {
        rx.join().unwrap_or_else(|err| {
            eprintln!("Encountered error while closing the logger: {:?}", err);
        });
    }

    let final_msg = LogInfo {
        message: String::from("Shutting down the logging service..."),
        client: None,
        level: InfoLevel::Info,
        time: Utc::now(),
    };


    if let Some(chan) = unsafe { CHAN.take() } {
        if let Err(SendError(msg)) = chan.0.send(LogMessage::Info(final_msg)) {
            debug::print("Failed to log the final message", debug::InfoLevel::Warning);
        }
    }

    // Need to block because otherwise the lazy_static contents may go expired too soon
    //    dump_log();
}

fn run<T>(writer: Box<T>)
where
    T: LogWriter + Send + Sync + 'static,
{
    let mut store: Vec<LogInfo> = Vec::with_capacity(1024);
    let writer = Arc::new(writer);
    let rx = unsafe { &CHAN.as_ref().unwrap().1 };

    for info in rx {
        match info {
            LogMessage::Info(i) => store.push(i),
            LogMessage::Dump => {
                let arc_writer = Arc::clone(&writer);
                let mut dump_store = Vec::with_capacity(1024);
                mem::swap(&mut store, &mut dump_store);

                thread::spawn(move || {
                    dump_log(dump_store, arc_writer);
                });
            }
            LogMessage::Shutdown => break,
        }
    }
}

fn start_refresh(period: Duration) {
    unsafe {
        if REFRESH_HANDLER.is_some() {
            stop_refresh();
        }

        REFRESH_HANDLER = Some(thread::spawn(move || loop {
            thread::sleep(period);
            thread::spawn(|| {
                //                dump_log();
            });
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

        let config =
            match unsafe { CONFIG.as_mut() } {
                Ok(c) => c,
                Err(_) => return,
            };

        if let Some(p) = period {
            config.set_refresh_period(p);
        }

        start_refresh(config.get_refresh_period());
    });
}

fn dump_log<T>(data: Vec<LogInfo>, writer: Arc<Box<T>>)
where
    T: LogWriter + Send + Sync,
{
    if data.is_empty() {
        return;
    }

    while DUMP_IN_PROGRESS
        .compare_exchange_weak(false, true, Ordering::SeqCst, Ordering::Relaxed)
        .is_err()
    {
        cpu_relax(8);
    }

    let mut remainder = data;

    while let Err(pos) = writer.dump(&remainder) {
        let mut index = 0;
        remainder.retain(|_| {
            if index < pos {
                return false;
            }

            index += 1;
            true
        });
    }

    DUMP_IN_PROGRESS.store(false, Ordering::SeqCst);
}

fn write_to_file(file: &mut File, content: &str) {
    file.write_all(content.as_bytes()).unwrap_or_else(|err| {
        eprintln!("Failed to write to dump file: {}...", err);
    });
}

fn format_content(level: &InfoLevel, message: &str, timestamp: DateTime<Utc>) -> String {
    [
        "\r\n[",
        &level.to_string(),
        "] @ ",
        &timestamp.to_rfc3339(),
        ": ",
        message,
    ]
    .join("")
}

fn create_dump_file(id: String, loc: &PathBuf) -> Result<File, String> {
    if !loc.as_path().is_dir() {
        if let Err(e) = fs::create_dir_all(loc) {
            return Err(format!("Failed to dump the logging information: {}", e));
        }
    }

    let base = if id.is_empty() {
        [&Utc::now().to_string(), ".txt"].join("")
    } else {
        [&id, "-", &Utc::now().to_string(), ".txt"].join("")
    };

    let mut path = loc.to_owned();
    path.push(base);

    match File::create(path.as_path()) {
        Ok(file) => Ok(file),
        _ => Err(String::from("Unable to create the dump file")),
    }
}
