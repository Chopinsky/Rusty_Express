use std::collections::HashMap;
use std::time::Duration;
use core::http::*;

pub struct ServerConfig {
    pub pool_size: usize,
    pub read_timeout: u8,
    pub write_timeout: u8,
    pub use_session: bool,
    session_auto_clean_period: Option<Duration>,
    meta_data: ConnMetadata,
}

impl ServerConfig {
    pub fn new() -> Self {
        ServerConfig {
            pool_size: 8,
            read_timeout: 8,
            write_timeout: 8,
            use_session: false,
            session_auto_clean_period: Some(Duration::new(3600, 0)),
            meta_data: ConnMetadata::new(),
        }
    }

    pub fn get_meta_data(&self) -> ConnMetadata {
        self.meta_data.to_owned()
    }

    pub fn use_default_header(&mut self, header: &HashMap<String, String>) {
        self.meta_data.header = header.clone();
    }

    pub fn set_default_header(&mut self, field: String, value: String, replace: bool) {
        set_header(&mut self.meta_data.header, field, value, replace);
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
    pub fn new() -> Self {
        ConnMetadata {
            header: HashMap::new(),
            default_pages: HashMap::new(),
        }
    }

    pub fn get_default_header(&self) -> HashMap<String, String> {
        self.header.to_owned()
    }

    pub fn get_default_pages(&self) -> HashMap<u16, String> {
        self.default_pages.to_owned()
    }
}

impl Clone for ConnMetadata {
    fn clone(&self) -> Self {
        ConnMetadata {
            header: self.header.clone(),
            default_pages: self.default_pages.clone(),
        }
    }
}