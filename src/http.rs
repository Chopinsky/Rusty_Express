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
    status: String,
    body: String,
}

impl Response {
    pub fn new() -> Self {
        Response {
            status: String::new(),
            body: String::new(),
        }
    }

    pub fn write(&mut self, content: String) {
        if !content.is_empty() {
            self.body.push_str(&content);
        }
    }

    pub fn get_status(status: u16) -> String {
        let status_base =
            match status {
                200 => "200 OK",
                500 => "500 INTERNAL SERVER ERROR",
                400 => "400 BAD REQUEST",
                404 | _ => "404 NOT FOUND",
            };

        return format!("HTTP/1.1 {}\r\n\r\n", status_base);
    }
}

