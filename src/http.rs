#![allow(dead_code)]

use std::collections::HashMap;
use std::collections::hash_map::Iter;
use std::fs::File;
use std::io::BufReader;
use std::io::prelude::*;
use std::path::Path;
use chrono::prelude::*;
use router::REST;

static FOUR_OH_FOUR: &'static str = include_str!("./default/404.html");
static FIVE_HUNDRED: &'static str = include_str!("./default/500.html");

pub struct Request {
    pub method: REST,
    pub path: String,
    scheme: HashMap<String, Vec<String>>,
    cookie: HashMap<String, String>,
    header: HashMap<String, String>,
    body: Vec<String>,
}

impl Request {
    pub fn build_from(
        method: REST,
        path: String,
        scheme: HashMap<String, Vec<String>>,
        cookie: HashMap<String, String>,
        header: HashMap<String, String>,
        body: Vec<String>
    ) -> Self {
        Request {
            method,
            path,
            cookie,
            scheme,
            header,
            body,
        }
    }

    pub fn header(&self, field: &str) -> Option<String> {
        if field.is_empty() { return None; }
        if self.header.is_empty() { return None; }

        match self.header.get(&field[..]) {
            Some(value) => Some(value.to_owned()),
            None => None,
        }
    }

    pub fn cookie(&self, key: &str) -> Option<String> {
        if key.is_empty() { return None; }
        if self.cookie.is_empty() { return None; }

        match self.cookie.get(&key[..]) {
            Some(value) => Some(value.to_owned()),
            None => None,
        }
    }

    pub fn cookie_iter(&self) -> Iter<String, String> {
        self.cookie.iter()
    }

    pub fn scheme(&self, field: &str) -> Option<Vec<String>> {
        if field.is_empty() { return None; }
        if self.scheme.is_empty() { return None; }

        match self.scheme.get(&field[..]) {
            Some(value) => Some(value.to_owned()),
            None => None,
        }
    }
}

pub struct Response {
    status: u16,
    content_type: String,
    cookie: String,
    header: HashMap<String, String>,
    body: String,
}

pub trait ResponseWriter {
    fn send(&mut self, content: String);
    fn send_file(&mut self, file_path: String);
    fn set_cookies(&mut self, cookie: HashMap<String, String>);
    fn set_content_type(&mut self, content_type: String);
}

impl Response {
    pub fn new() -> Self {
        Response {
            status: 0,
            content_type: String::new(),
            cookie: String::new(),
            header: HashMap::new(),
            body: String::new(),
        }
    }

    pub fn new_with_default_header(default_header: &HashMap<String, String>) -> Self {
        Response {
            status: 0,
            content_type: String::new(),
            cookie: String::new(),
            header: default_header.clone(),
            body: String::new(),
        }
    }

    pub fn status(&mut self, status: u16) {
        self.status =
            match status {
                100 ... 101 => status,
                200 ... 206 => status,
                300 ... 308 if status != 307 && status != 308 => status,
                400 ... 417 if status != 402 => status,
                426 | 428 | 429 | 431 | 451 => status,
                500 ... 505 | 511 => status,
                _ => 0,
            };
    }

    pub fn header(&mut self, field: String, value: String, replace: bool) {
        set_header(&mut self.header, field, value, replace);
    }

    pub fn status_is_set(&self) -> bool {
        (self.status == 0)
    }

    pub fn has_contents(&self) -> bool {
        (!self.body.is_empty() && self.body.len() > 0)
    }

    pub fn check_and_update(&mut self, fallback: &HashMap<u16, String>) {
        //if contents have been provided, we're all good.
        if self.has_contents() { return; }

        if self.status == 0 || self.status == 404 {
            if let Some(file_path) = fallback.get(&404) {
                let (_, content) = read_from_file(Path::new(file_path));
                if !content.is_empty() { self.body.push_str(&content); }
            } else {
                self.body.push_str(FOUR_OH_FOUR);
            }
        } else {
            if let Some(file_path) = fallback.get(&500) {
                let (_, content) = read_from_file(Path::new(file_path));
                if !content.is_empty() { self.body.push_str(&content); }
            } else {
                self.body.push_str(FIVE_HUNDRED);
            }
        }
    }

    pub fn serialize(&self) -> String {
        let mut result= String::new();

        result.push_str(&self.get_header());

        if self.has_contents() {
            //content has been explicitly set, use them
            result.push_str(&self.body);
        } else if self.status == 404 || self.status == 500 {
            //explicit error status
            result.push_str(&get_default_page(self.status));
        } else if self.status == 0 {
            //implicit error status
            result.push_str(&get_default_page(404));
        }

        result
    }

    fn get_header(&self) -> String {
        let mut header = String::new();

        match self.status {
            404 | 500 => {
                header.push_str(&get_status(self.status));
            },
            0 => {
                /* No status has been explicitly set, be smart here */
                if self.has_contents() {
                    header.push_str(&get_status(200));
                } else {
                    header.push_str(&get_status(404));
                }
            },
            _ => {
                /* A status has been set explicitly, respect that here. */
                header.push_str(&get_status(self.status));
            },
        }

        if !self.content_type.is_empty() {
            header.push_str(&format!("Content-Type: {}\r\n", self.content_type));
        }

        if !self.cookie.is_empty() {
            header.push_str(&format!("Set-Cookie: {}\r\n", self.cookie));
        }

        if !self.header.contains_key("date") {
            let dt = Local::now();
            header.push_str(&format!("Date: {}\r\n", dt.to_rfc2822()));
        }

        //other header field-value pairs
        for (field, value) in self.header.iter() {
            //special cases that shall be set using given methods
            let f = field.to_lowercase();
            if f.eq("content-type") || f.eq("set-cookie") || f.eq("date") {
                continue;
            }

            //otherwise, write to the header
            header.push_str(&format!("{}: {}\r\n", field, value));
        }

        //write an empty line to end the header
        header.push_str("\r\n");
        header
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
        if !file_path.is_file() {
            // if doesn't exist or not a file, fail now
            println!("Can't locate requested file");
            self.status(404);
        } else {
            let (status, contents) = read_from_file(&file_path);

            if !self.status_is_set() { self.status(status); }
            if !contents.is_empty() { self.body.push_str(&contents); }

            if self.status == 200 && self.content_type.is_empty() {
                self.set_content_type(default_content_type_on_ext(&file_path));
            }
        }
    }

    fn set_cookies(&mut self, cookie: HashMap<String, String>) {
        if !cookie.is_empty() {
            // pair data structure: (key, value)
            for (key, val) in cookie.iter() {
                //if key is empty, skip
                if key.is_empty() { continue; }
                //if multiple cookies, set delimiter ";"
                if !self.cookie.is_empty() { self.cookie.push_str(&"; "); }

                if val.is_empty() {
                    //if no value, then only set the key
                    self.cookie.push_str(key);
                } else {
                    //if a key-value pair, then set the pair
                    self.cookie.push_str(&format!("{}={}", key, val));
                }
            }
        }
    }

    fn set_content_type(&mut self, content_type: String) {
        if !content_type.is_empty() {
            self.content_type = content_type;
        }
    }
}

pub fn set_header(header: &mut HashMap<String, String>, field: String, value: String, replace: bool) {
    if field.is_empty() || value.is_empty() { return; }

    let f = field.to_lowercase();
    if !header.contains_key(&f) {
        //new field, insert
        header.insert(f, value);
    } else if let Some(store) = header.get_mut(&f) {
        //existing field, replace existing value or append depending on the parameter
        if replace {
            *store = value;
        } else {
            *store = format!("{}; {}", store, value);
        }
    }
}

fn get_default_page(status: u16) -> String {
    match status {
        500 => {
            /* return default 500 page */
            String::from(FIVE_HUNDRED)
        },
        404 => {
            /* return default/override 404 page */
            String::from(FOUR_OH_FOUR)
        },
        _ => {
            /* return 404 page for now */
            String::from(FOUR_OH_FOUR)
        }
    }
}

fn read_from_file(file_path: &Path) -> (u16, String) {
    // try open the file
    if let Ok(file) = File::open(file_path) {
        let mut buf_reader = BufReader::new(file);
        let mut contents: String = String::new();

        return match buf_reader.read_to_string(&mut contents) {
            Err(e) => {
                println!("Unable to read file: {}", e);
                (500, String::new())
            },
            Ok(_) if contents.len() > 0 => {
                //things are truly ok now
                (200, contents)
            },
            _ => {
                println!("File stream finds nothing...");
                (404, String::new())
            }
        };
    } else {
        println!("Unable to open requested file for path");
        (404, String::new())
    }
}

fn get_status(status: u16) -> String {
    let status_base =
        match status {
            200 => "200 OK",
            500 => "500 INTERNAL SERVER ERROR",
            400 => "400 BAD REQUEST",
            404 | _ => "404 NOT FOUND",
        };

    return format!("HTTP/1.1 {}\r\n", status_base);
}

fn default_content_type_on_ext(path: &Path) -> String {
    if let Some(ext) = path.extension() {
        match ext.to_str() {
            Some("css") | Some("scss") | Some("sass") | Some("less") => String::from("text/css"),
            Some("js") | Some("ts") | Some("jsx") => String::from("application/javascript"),
            Some("html") => String::from("text/html"),
            Some("jpeg") | Some("gif") | Some("png") | Some("bmp") | Some("webp") => {
                format!("image/{}", ext.to_string_lossy())
            },
            Some("midi") | Some("mp3") => {
                format!("audio/{}", ext.to_string_lossy())
            },
            Some("webm") | Some("mp4") | Some("ogg") | Some("wav") => {
                format!("video/{}", ext.to_string_lossy())
            },
            Some("xml") | Some("xhtml") | Some("pdf") => {
                format!("application/{}", ext.to_string_lossy())
            },
            _ => String::from("text/plain"),
        }
    } else {
        String::from("text/plain")
    }
}

