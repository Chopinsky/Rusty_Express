#![allow(dead_code)]

use std::collections::HashMap;
use router::REST;

pub struct Request {
    pub method: REST,
    pub path: String,
    pub header: HashMap<String, String>,
}

pub struct Response {
    header: String,
    body: String,
}

impl Request {
    pub fn new() -> Request {
        Request {
            method: REST::NONE,
            path: String::new(),
            header: HashMap::new(),
        }
    }

    pub fn build_from(method: REST,
                      path: String,
                      header: HashMap<String, String>) -> Request {
        Request {
            method,
            path,
            header,
        }
    }
}
