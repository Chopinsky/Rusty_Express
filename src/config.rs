use std::collections::HashMap;
use http::*;

pub struct ServerConfig {
    pub pool_size: usize,
    pub read_timeout: u8,
    pub write_timeout: u8,
    header: HashMap<String, String>,
}

impl ServerConfig {
    pub fn new() -> Self {
        ServerConfig {
            pool_size: 4,
            read_timeout: 5,
            write_timeout: 5,
            header: HashMap::new(),
        }
    }

    pub fn use_default_header(&mut self, header: &HashMap<String, String>) {
        self.header = header.clone();
    }

    pub fn default_header(&mut self, field: String, value: String, replace: bool) {
        set_header(&mut self.header, field, value, replace);
    }

    pub fn get_default_header(&self) -> HashMap<String, String> {
        self.header.clone()
    }
}