#![allow(dead_code)]

use std::collections::HashMap;
use std::path::Path;
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
    status: u16,
    body: String,
}

impl Response {
    pub fn new() -> Self {
        Response {
            status: 0,
            body: String::new(),
        }
    }

    pub fn status(&mut self, status: u16) {
        self.status =
            match status {
                200 ... 206 => status,
                300 ... 308 if status != 307 && status != 308 => status,
                400 ... 417 if status != 402 => status,
                426 | 428 | 429 | 431 | 451 => status,
                500 ... 505 | 511 => status,
                _ => 0,
            };
    }

    pub fn send(&mut self, content: String) {
        if !content.is_empty() {
            self.body.push_str(&content);
        }
    }

    pub fn send_file(&mut self, file_path: String) {
        if file_path.is_empty() {
            println!("Undefined file path to retrieve data from...");
            return;
        }

        let path = Path::new(&file_path[..]);

    }

    pub fn serialize(&self) -> String {
        self.body.to_owned()
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

