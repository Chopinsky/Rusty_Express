use std::collections::HashMap;
use std::time::Duration;
use http::*;

pub struct ServerConfig {
    pub pool_size: usize,
    pub read_timeout: u8,
    pub write_timeout: u8,
    session_auto_clean_period: Option<Duration>,
    header: HashMap<String, String>,
    default_pages: HashMap<u16, String>,
}

impl ServerConfig {
    pub fn new() -> Self {
        ServerConfig {
            pool_size: 4,
            read_timeout: 4,
            write_timeout: 4,
            session_auto_clean_period: Some(Duration::new(3600, 0)),
            header: HashMap::new(),
            default_pages: HashMap::new(),
        }
    }

    pub fn use_default_header(&mut self, header: &HashMap<String, String>) {
        self.header = header.clone();
    }

    pub fn default_header(&mut self, field: String, value: String, replace: bool) {
        set_header(&mut self.header, field, value, replace);
    }

    pub fn enable_session_auto_clean(&mut self, auto_clean_period: Duration) {
        self.session_auto_clean_period = Some(auto_clean_period);
    }

    pub fn disable_session_auto_clean(&mut self) {
        self.session_auto_clean_period = None;
    }

    pub fn get_session_auto_clean_period(&self) -> Option<Duration> {
        self.session_auto_clean_period
    }
}

pub struct ConnMetadata {
    header: HashMap<String, String>,
    default_pages: HashMap<u16, String>,
}

impl ConnMetadata {
    pub fn from(config: &ServerConfig) -> Self {
        ConnMetadata {
            header: config.header.to_owned(),
            default_pages: config.default_pages.to_owned(),
        }
    }

    pub fn get_default_header(&self) -> HashMap<String, String> {
        self.header.to_owned()
    }

    pub fn get_default_pages(&self) -> HashMap<u16, String> {
        self.default_pages.to_owned()
    }
}