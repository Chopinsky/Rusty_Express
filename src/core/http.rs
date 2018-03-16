#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_imports)]

use std::collections::HashMap;
use std::collections::hash_map::Iter;
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::io::prelude::*;
use std::net::{TcpStream};
use std::path::Path;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use chrono::prelude::*;
use core::cookie::*;
use core::router::REST;
use core::config::{EngineContext, ServerConfig, ViewEngine, ViewEngineParser};
use support::common::MapUpdates;
use support::shared_pool;
use support::TaskType;

static FOUR_OH_FOUR: &'static str = include_str!("../default/404.html");
static FIVE_HUNDRED: &'static str = include_str!("../default/500.html");
static VERSION: &'static str = "0.2.9";

pub struct Request {
    pub method: REST,
    pub uri: String,
    cookie: HashMap<String, String>,
    scheme: HashMap<String, Vec<String>>,
    params: HashMap<String, String>,
    header: HashMap<String, String>,
    body: Vec<String>,
}

impl Request {
    pub fn new() -> Self {
        Request {
            method: REST::GET,
            uri: String::new(),
            cookie: HashMap::new(),
            scheme: HashMap::new(),
            params: HashMap::new(),
            header: HashMap::new(),
            body: Vec::new(),
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

    #[inline]
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

    #[inline]
    pub fn param(&self, key: &str) -> Option<&String> {
        self.params.get(key)
    }

    #[inline]
    pub fn param_iter(&self) -> Iter<String, String> {
        self.params.iter()
    }
}

pub trait RequestWriter {
    fn write_header(&mut self, key: &str, val: &str, allow_override: bool);
    fn write_scheme(&mut self, key: &str, val: Vec<String>, allow_override: bool);
    fn create_scheme(&mut self, scheme: HashMap<String, Vec<String>>);
    fn set_cookie(&mut self, key: &str, val: &str, allow_override: bool);
    fn create_cookie(&mut self, cookie: HashMap<String, String>);
    fn set_param(&mut self, key: &str, val: &str);
    fn create_param(&mut self, params: HashMap<String, String>);
    fn extend_body(&mut self, content: &str);
}

impl RequestWriter for Request {
    fn write_header(&mut self, key: &str, val: &str, allow_override: bool) {
        self.header.add(key, val.to_owned(), allow_override);
    }

    fn write_scheme(&mut self, key: &str, val: Vec<String>, allow_override: bool) {
        self.scheme.add(key, val.to_owned(), allow_override);
    }

    fn create_scheme(&mut self, scheme: HashMap<String, Vec<String>>) {
        self.scheme = scheme;
    }

    fn set_cookie(&mut self, key: &str, val: &str, allow_override: bool) {
        self.cookie.add(key, val.to_owned(), allow_override);
    }

    fn create_cookie(&mut self, cookie: HashMap<String, String>) {
        self.cookie = cookie;
    }

    #[inline]
    fn set_param(&mut self, key: &str, val: &str) {
        self.params.entry(key.to_owned()).or_insert(val.to_owned());
    }

    #[inline]
    fn create_param(&mut self, params: HashMap<String, String>) {
        self.params = params;
    }

    fn extend_body(&mut self, content: &str) {
        self.body.push(content.to_owned());
    }
}

pub struct Response {
    status: u16,
    keep_alive: bool,
    content_type: String,
    cookie: Arc<HashMap<String, Cookie>>,
    header: HashMap<String, String>,
    body: Box<String>,
    body_rx: Vec<mpsc::Sender<String>>,
    redirect: String,
}

impl Response {
    pub fn new() -> Self {
        Response {
            status: 0,
            keep_alive: false,
            content_type: String::new(),
            cookie: Arc::new(HashMap::new()),
            header: HashMap::new(),
            body: Box::new(String::new()),
            body_rx: Vec::new(),
            redirect: String::new(),
        }
    }

    pub fn new_with_default_header(default_header: HashMap<String, String>) -> Self {
        Response {
            status: 0,
            keep_alive: false,
            content_type: String::new(),
            cookie: Arc::new(HashMap::new()),
            header: default_header,
            body: Box::new(String::new()),
            body_rx: Vec::new(),
            redirect: String::new(),
        }
    }

    fn resp_header(&self, ignore_body: bool) -> Box<String> {
        // Get cookier parser to its own thread
        let (tx, rx) = mpsc::channel();
        if !self.cookie.is_empty() {
            let cookie = Arc::clone(&self.cookie);

            shared_pool::run(move || {
                write_header_cookie(cookie, tx);
            }, TaskType::Response);

        } else {
            drop(tx);
        }

        let mut header =
            Box::new(write_header_status(self.status, self.has_contents(ignore_body)));

        // other header field-value pairs
        write_headers(&self.header, &mut header);

        if !self.content_type.is_empty() {
            header.push_str(&format!("Content-Type: {}\r\n", self.content_type));
        }

        if !self.header.contains_key("date") {
            let dt = Utc::now();
            header.push_str(&format!("Date: {}\r\n", dt.format("%a, %e %b %Y %T GMT").to_string()));
        }

        if !self.header.contains_key("content-length") {
            //TODO: if using rx, need to move this to body writer, and don't append empty line in this function but in the body writer function.
            if ignore_body || self.body.is_empty() {
                header.push_str("Content-Length: 0\r\n");
            } else {
                header.push_str(&format!("Content-Length: {}\r\n", self.body.len()));
            }
        }

        if !self.header.contains_key("connection") {
            let connection =
                if self.keep_alive {
                    "keep-alive"
                } else {
                    "close"
                };

            header.push_str(&format!("Connection: {}\r\n", connection));
        }

        if let Ok(received ) = rx.recv_timeout(Duration::from_millis(64)) {
            if !received.is_empty() {
                header.push_str(&received);
            }
        }

        // write an empty line to end the header
        header.push_str("\r\n");
        header
    }
}

pub trait ResponseStates {
    fn to_keep_alive(&self) -> bool;
    fn get_redirect_path(&self) -> String;
    fn get_header(&self, key: &str) -> Option<&String>;
    fn get_cookie(&self, key: &str) -> Option<&Cookie>;
    fn get_content_type(&self) -> String;
    fn status_is_set(&self) -> bool;
    fn has_contents(&self, ignore_body: bool) -> bool;
}

impl ResponseStates for Response {
    #[inline]
    fn to_keep_alive(&self) -> bool {
        self.keep_alive
    }

    #[inline]
    fn get_redirect_path(&self) -> String {
        self.redirect.to_owned()
    }

    #[inline]
    fn get_header(&self, key: &str) -> Option<&String> {
        self.header.get(key)
    }

    #[inline]
    fn get_cookie(&self, key: &str) -> Option<&Cookie> {
        self.cookie.get(key)
    }

    #[inline]
    fn get_content_type(&self) -> String {
        self.content_type.to_owned()
    }

    fn status_is_set(&self) -> bool {
        match self.status {
            0 => false,
            _ => true,
        }
    }

    #[inline]
    fn has_contents(&self, ignore_body: bool) -> bool {
        (ignore_body || !self.body.is_empty())
    }
}

pub trait ResponseWriter {
    fn status(&mut self, status: u16);
    fn header(&mut self, field: &str, value: &str, replace: bool);
    fn send(&mut self, content: &str);
    fn send_file(&mut self, file_path: &str);
    fn send_template(&mut self, file_path: &str, context: Box<EngineContext>);
    fn set_cookie(&mut self, cookie: Cookie);
    fn set_cookies(&mut self, cookie: &[Cookie]);
    fn clear_cookies(&mut self);
    fn set_content_type(&mut self, content_type: &str);
    fn check_and_update(&mut self, fallback: &HashMap<u16, String>);
    fn keep_alive(&mut self, to_keep: bool);
    fn redirect(&mut self, path: &str);
}

impl ResponseWriter for Response {
    fn status(&mut self, status: u16) {
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

    fn header(&mut self, field: &str, value: &str, replace: bool) {
        if field.is_empty() || value.is_empty() { return; }

        match &field.to_lowercase()[..] {
            "content-type" => self.content_type = value.to_owned(),
            "connection" => {
                if value.to_lowercase().eq("keep-alive") {
                    self.keep_alive = true;
                }
            },
            _ => {
                self.header.add(field, value.to_owned(), replace);
            },
        };
    }

    fn send(&mut self, content: &str) {
        if !content.is_empty() {
            self.body.push_str(content);
        }
    }

    fn send_file(&mut self, file_loc: &str) {

        //TODO - use meta path
        //TODO - send back rx, and write to stream from receiver

        if file_loc.is_empty() {
            eprintln!("Undefined file path to retrieve data from...");
            return;
        }

        let file_path = Path::new(file_loc);
        if !file_path.is_file() {
            eprintln!("Can't locate requested file");
            self.status(404);
            return;
        }

        let status = read_from_file(&file_path, &mut self.body);
        if !self.status_is_set() { self.status(status); }

        if self.status == 200 && self.content_type.is_empty() {
            let mime_type =
                if let Some(ext) = file_path.extension() {
                    let file_extension = ext.to_string_lossy();
                    default_mime_type_with_ext(&file_extension)
                } else {
                    String::from("text/plain")
                };

            self.set_content_type(&mime_type[..]);
        }
    }

    fn send_template(&mut self, file_loc: &str, context: Box<EngineContext>) {
        //TODO - impl ServerConfig::template_parser
        //ServerConfig::template_parser();
    }

    fn set_cookie(&mut self, cookie: Cookie) {
        if !cookie.is_valid() { return; }

        if let Some(cookie_set) = Arc::get_mut(&mut self.cookie) {
            let key = cookie.get_cookie_key();
            cookie_set.insert(key, cookie);
        }
    }

    fn set_cookies(&mut self, cookies: &[Cookie]) {
        if let Some(cookie_set) = Arc::get_mut(&mut self.cookie) {
            for cookie in cookies.iter() {
                if !cookie.is_valid() { continue; }

                let key = cookie.get_cookie_key();
                cookie_set.insert(key, cookie.clone());
            }
        }
    }

    fn clear_cookies(&mut self) {
        if let Some(cookie_set) = Arc::get_mut(&mut self.cookie) {
            cookie_set.clear();
        }
    }

    fn set_content_type(&mut self, content_type: &str) {
        if !content_type.is_empty() {
            self.content_type = content_type.to_owned();
        }
    }

    fn check_and_update(&mut self, fallback: &HashMap<u16, String>) {
        //if contents have been provided, we're all good.
        if self.has_contents(false) { return; }

        if self.status == 0 || self.status == 404 {
            if let Some(file_path) = fallback.get(&404) {
                read_from_file(Path::new(file_path), &mut self.body);
            } else {
                self.body = Box::new(FOUR_OH_FOUR.to_owned());
            }
        } else {
            if let Some(file_path) = fallback.get(&500) {
                read_from_file(Path::new(file_path), &mut self.body);
            } else {
                self.body = Box::new(FIVE_HUNDRED.to_owned());
            }
        }
    }

    fn keep_alive(&mut self, to_keep: bool) {
        self.keep_alive = to_keep;
    }

    /// Can only redirect to internal path, no outsource path, sorry for the hackers (FYI, you can
    /// still hack the redirection link via Javascript)!
    fn redirect(&mut self, path: &str) {
        self.redirect = path.to_owned();
    }
}

pub trait StreamWriter {
    fn serialize_header(&self, buffer: &mut BufWriter<&TcpStream>, ignore_body: bool);
    fn serialize_body(&self, buffer: &mut BufWriter<&TcpStream>);
}

impl StreamWriter for Response {
    fn serialize_header(&self, buffer: &mut BufWriter<&TcpStream>, ignore_body: bool) {
        if let Err(e) = buffer.write(self.resp_header(ignore_body).as_bytes()) {
            eprintln!("An error has taken place when writing the response header to the stream: {}", e);
        }
    }

    fn serialize_body(&self, buffer: &mut BufWriter<&TcpStream>) {
        if self.has_contents(false) {
            //content has been explicitly set, use them
            if let Err(e) = buffer.write(self.body.as_bytes()) {
                eprintln!("An error has taken place when writing the response header to the stream: {}", e);
            }
        } else {
            match self.status {
                //explicit error status
                0 | 404 => get_default_page(buffer, 404),
                500 => get_default_page(buffer, 500),
                _ => { /* Nothing */ },
            };
        }
    }
}

fn get_default_page(buffer: &mut BufWriter<&TcpStream>, status: u16) {
    match status {
        500 => {
            /* return default 500 page */
            if let Err(e) = buffer.write(FIVE_HUNDRED.as_bytes()) {
                eprintln!("An error has taken place when writing the response body to the stream: {}", e);
            }
        },
        _ => {
            /* return default/override 404 page */
            if let Err(e) = buffer.write(FOUR_OH_FOUR.as_bytes()) {
                eprintln!("An error has taken place when writing the response body to the stream: {}", e);
            }
        },
    }
}

fn read_from_file(file_path: &Path, buf: &mut Box<String>) -> u16 {
    // try open the file
    if let Ok(file) = File::open(file_path) {
        let mut buf_reader = BufReader::new(file);
        return match buf_reader.read_to_string(buf) {
            Err(e) => {
                eprintln!("Unable to read file: {}", e);
                500
            },
            Ok(size) if size > 0 => {
                //things are truly ok now
                200
            },
            _ => {
                eprintln!("File stream finds nothing...");
                404
            }
        };
    } else {
        eprintln!("Unable to open requested file for path");
        404
    }
}

fn get_status(status: u16) -> String {
    let status_base =
        match status {
            100 => "100 Continue",
            101 => "101 Switching Protocols",
            200 => "200 OK",
            201 => "201 Created",
            202 => "202 Accepted",
            203 => "203 Non-Authoritative Information",
            204 => "204 No Content",
            205 => "205 Reset Content",
            206 => "206 Partial Content",
            300 => "Multiple Choices",
            301 => "301 Moved Permanently",
            302 => "302 Found",
            303 => "303 See Other",
            304 => "304 Not Modified",
            307 => "307 Temporary Redirect",
            308 => "308 Permanent Redirect",
            400 => "400 Bad Request",
            401 => "401 Unauthorized",
            403 => "403 Forbidden",
            404 => "404 Not Found",
            405 => "405 Method Not Allowed",
            406 => "406 Not Acceptable",
            407 => "407 Proxy Authentication Required",
            408 => "408 Request Timeout",
            409 => "409 Conflict",
            410 => "410 Gone",
            411 => "411 Length Required",
            412 => "412 Precondition Failed",
            413 => "413 Payload Too Large",
            414 => "414 URI Too Long",
            415 => "415 Unsupported Media Type",
            416 => "416 Range Not Satisfiable",
            417 => "417 Expectation Failed",
            426 => "426 Upgrade Required",
            428 => "428 Precondition Required",
            429 => "429 Too Many Requests",
            431 => "431 Request Header Fields Too Large",
            451 => "451 Unavailable For Legal Reasons",
            500 => "500 Internal Server Error",
            501 => "501 Not Implemented",
            502 => "502 Bad Gateway",
            503 => "503 Service Unavailable",
            504 => "504 Gateway Timeout",
            505 => "505 HTTP Version Not Supported",
            511 => "511 Network Authentication Required",
            _ => "403 Forbidden",
        };

    return format!("HTTP/1.1 {}\r\n", status_base);
}

fn default_mime_type_with_ext(ext: &str) -> String {
    match ext {
        "abw" => String::from("application/x-abiword"),
        "arc" | "bin" => String::from("application/octet-stream"),
        "avi" => String::from("video/x-msvideo"),
        "azw" => String::from("application/vnd.amazon.ebook"),
        "bz" => String::from("application/x-bzip"),
        "bz2" => String::from("application/x-bzip2"),
        "css" | "scss" | "sass" | "less" => String::from("text/css"),
        "doc" => String::from("application/msword"),
        "docx" => String::from("application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
        "eot" => String::from("application/vnd.ms-fontobject"),
        "epub" => String::from("application/epub+zip"),
        "js" | "jsx" => String::from("application/javascript"),
        "ts" => String::from("application/typescript"),
        "ico" => String::from("image/x-icon"),
        "ics" => String::from("text/calendar"),
        "jar" => String::from("application/java-archive"),
        "mpkg" => String::from("application/vnd.apple.installer+xml"),
        "odp" => String::from("application/vnd.oasis.opendocument.presentation"),
        "ods" => String::from("application/vnd.oasis.opendocument.spreadsheet"),
        "odt" => String::from("application/vnd.oasis.opendocument.text"),
        "ppt" => String::from("application/vnd.ms-powerpoint"),
        "pptx" => String::from("application/vnd.openxmlformats-officedocument.presentationml.presentation"),
        "rar" => String::from("application/x-rar-compressed"),
        "swf" => String::from("application/x-shockwave-flash"),
        "vsd" => String::from("application/vnd.visio"),
        "wasm" => String::from("application/wasm"),
        "weba" => String::from("audio/webm"),
        "xhtml" => String::from("application/xhtml+xml"),
        "xul" => String::from("application/vnd.mozilla.xul+xml"),
        "7z" => String::from("application/x-7z-compressed"),
        "svg" => String::from("image/svg+xml"),
        "csh" | "sh" | "tar" | "wav" => format!("application/x-{}", ext),
        "csv" | "html" | "htm" => format!("text/{}", ext),
        "jpeg" | "jpg" | "gif" | "png" | "bmp" | "webp" | "tiff" | "tif" => format!("image/{}", ext),
        "otf" | "ttf" | "woff" | "woff2" => format!("font/{}", ext),
        "midi" | "mp3" | "aac" | "mid" | "oga"  => format!("audio/{}", ext),
        "webm" | "mp4" | "ogg" | "mpeg" | "ogv" => format!("video/{}", ext),
        "xml" | "pdf" | "json" | "ogx" | "rtf" | "zip" => format!("application/{}", ext),
        _ if !ext.is_empty() => format!("application/{}", ext),
        _ => String::from("text/plain"),
    }
}

fn write_header_status(status: u16, has_contents: bool) -> String {
    match status {
        404 | 500 => {
            get_status(status)
        },
        0 => {
            /* No status has been explicitly set, be smart here */
            if has_contents {
                get_status(200)
            } else {
                get_status(404)
            }
        },
        _ => {
            /* A status has been set explicitly, respect that here. */
            get_status(status)
        },
    }
}

fn write_headers(header: &HashMap<String, String>, final_header: &mut Box<String>) {
    final_header.push_str(&format!("Server: Rusty-Express/{}\r\n", VERSION));

    for (field, value) in header.iter() {
        final_header.push_str(&format!("{}: {}\r\n", field, value));
    }
}

fn write_header_cookie(cookie: Arc<HashMap<String, Cookie>>, tx: mpsc::Sender<String>) {
    let mut cookie_output = String::new();
    for (_, cookie) in cookie.iter() {
        if cookie.is_valid() {
            cookie_output.push_str(&format!("Set-Cookie: {}\r\n", &cookie.to_string()));
        }
    }

    tx.send(cookie_output).unwrap_or_else(|e| {
        eprintln!("Unable to write response cookies: {}", e);
    });
}

