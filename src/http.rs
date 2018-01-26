#![allow(dead_code)]

use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::io::prelude::*;
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

pub trait ResponseWriter {
    fn send(&mut self, content: String);
    fn send_file(&mut self, file_path: String);
}

impl Response {
    pub fn new() -> Self {
        Response {
            status: 0,
            body: String::new(),  // TODO: use string buffer instead of string literals
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

    pub fn status_is_set(&self) -> bool {
        (self.status == 0)
    }

    pub fn has_contents(&self) -> bool {
        (!self.body.is_empty() && self.body.len() > 0)
    }

    pub fn serialize(&self) -> String {
        let mut result= String::new();

        match self.status {
            404 | 500 => {
                return_default_page(self.status, &mut result);
            },
            0 => {
                /* No status has been explicitly set, be smart here */
                if self.has_contents() {
                    result.push_str(&Response::get_status(200));
                    result.push_str(&self.body);
                } else {
                    return_default_page(404, &mut result);
                }
            },
            _ => {
                /* A status has been set explicitly, respect that here. */
                result.push_str(&Response::get_status(self.status));
                if self.has_contents() {
                    result.push_str(&self.body);
                }
            },
        }

        result
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

impl ResponseWriter for Response {
    fn send(&mut self, content: String) {
        if !content.is_empty() {
            self.body.push_str(&content);
        }
    }

    fn send_file(&mut self, path: String) {
        if path.is_empty() {
            println!("Undefined file path to retrieve data from...");
            return;
        }

        let file_path = Path::new(&path);

        /* TODO: instead of write to response, write to self body */
        let mut response = String::new();
        if !file_path.is_file() {
            // if doesn't exist or not a file, fail now
            println!("Can't locate requested file");
            response = Response::get_status(404);
        } else {
            // try open the file
            if let Ok(file) = File::open(file_path) {
                let mut buf_reader = BufReader::new(file);
                let mut contents: String = String::new();

                match buf_reader.read_to_string(&mut contents) {
                    Err(e) => {
                        println!("Unable to read file: {} (requested path: {})", e, path);
                        response = Response::get_status(500);
                    },
                    Ok(_) if contents.len() > 0 => {
                        //things are truly ok now
                        //response.push_str(&status_line);
                        response.push_str(&contents);
                    },
                    _ => {
                        println!("File stream finds nothing...");
                        response = Response::get_status(404);
                    }
                }
            } else {
                println!("Unable to open requested file for path: {})", path);
                response = Response::get_status(404);
            }
        }
    }
}

fn return_default_page(status: u16, result: &mut String) {
    match status {
        500 => {
            result.push_str(&Response::get_status(500));
            /* return default/override 500 page */
        },
        _ => {
            result.push_str(&Response::get_status(404));
            /* return default/override 404 page */
        }
    }
}

