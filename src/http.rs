#![allow(dead_code)]

use std::collections::HashMap;
use router::REST;

pub struct Request {
    pub method: REST,
    pub path: String,
    header: HashMap<String, String>,
}

impl Request {
    pub fn build_from(method: REST,
                      path: String,
                      header: HashMap<String, String>) -> Self {
        Request {
            method,
            path,
            header,
        }
    }

    pub fn get(&self, key: String) -> Option<String> {
        if key.is_empty() { return None; }
        if self.header.is_empty() { return None; }

        match self.header.get(&key[..]) {
            Some(value) => Some(value.to_owned()),
            None => None,
        }
    }
}

pub struct Response {
    header: String,
    body: String,
}

