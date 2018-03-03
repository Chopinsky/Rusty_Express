#![allow(dead_code)]

use std::collections::HashMap;
use std::collections::hash_map::Iter;
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::io::prelude::*;
use std::net::{TcpStream};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use chrono::prelude::*;
use core::cookie::*;
use core::router::REST;

static FOUR_OH_FOUR: &'static str = include_str!("../default/404.html");
static FIVE_HUNDRED: &'static str = include_str!("../default/500.html");
static VERSION: &'static str = "0.2.8";

pub struct Request {
    pub method: Option<REST>,
    pub uri: String,
    cookie: HashMap<String, String>,
    scheme: HashMap<String, Vec<String>>,
    header: HashMap<String, String>,
    body: Vec<String>,
}

impl Request {
    pub fn build(
        method: Option<REST>,
        uri: String,
        scheme: HashMap<String, Vec<String>>,
        cookie: HashMap<String, String>,
        header: HashMap<String, String>,
        body: Vec<String>
    ) -> Self {
        Request {
            method,
            uri,
            cookie,
            scheme,
            header,
            body,
        }
    }

    pub fn new() -> Self {
        Request {
            method: None,
            uri: String::new(),
            cookie: HashMap::new(),
            scheme: HashMap::new(),
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

pub trait RequestWriter {
    fn write_header(&mut self, key: &str, val: &str, allow_override: bool);
    fn write_scheme(&mut self, key: &str, val: Vec<String>, allow_override: bool);
    fn create_scheme(&mut self, scheme: HashMap<String, Vec<String>>);
    fn set_cookie(&mut self, key: &str, val: &str, allow_override: bool);
    fn create_cookie(&mut self, cookie: HashMap<String, String>);
    fn extend_body(&mut self, content: &str);
}

impl RequestWriter for Request {
    fn write_header(&mut self, key: &str, val: &str, allow_override: bool) {
        set_header(&mut self.header, key.to_owned(), val.to_owned(), allow_override);
    }

    fn write_scheme(&mut self, key: &str, val: Vec<String>, allow_override: bool) {
        set_header(&mut self.scheme, key.to_owned(), val.to_owned(), allow_override);
    }

    fn create_scheme(&mut self, scheme: HashMap<String, Vec<String>>) {
        self.scheme = scheme;
    }

    fn set_cookie(&mut self, key: &str, val: &str, allow_override: bool) {
        set_header(&mut self.cookie, key.to_owned(), val.to_owned(), allow_override);
    }

    fn create_cookie(&mut self, cookie: HashMap<String, String>) {
        self.cookie = cookie;
    }

    fn extend_body(&mut self, content: &str) {
        self.body.push(content.to_owned());
    }
}

pub struct Response {
    status: u16,
    to_close: bool,
    content_type: String,
    cookie: HashMap<String, Cookie>,
    header: HashMap<String, String>,
    body: Box<String>,
    redirect: String,
}

impl Response {
    pub fn new() -> Self {
        Response {
            status: 0,
            to_close: false,
            content_type: String::new(),
            cookie: HashMap::new(),
            header: HashMap::new(),
            body: Box::new(String::new()),
            redirect: String::new(),
        }
    }

    pub fn new_with_default_header(default_header: &HashMap<String, String>) -> Self {
        Response {
            status: 0,
            to_close: false,
            content_type: String::new(),
            cookie: HashMap::new(),
            header: default_header.clone(),
            body: Box::new(String::new()),
            redirect: String::new(),
        }
    }

    fn resp_header(&self, ignore_body: bool) -> Box<String> {
        let status = self.status.to_owned();
        let has_contents = self.has_contents().to_owned();
        let (tx_status, rx_status) = mpsc::channel();

        thread::spawn(move || {
            // tx_core has been moved in, no need to drop specifically
            write_header_status(status, has_contents, tx_status);
        });

        // shared tx+rx for cookie + generic headers
        let (tx, rx) = mpsc::channel();

        // other header field-value pairs
        if !self.cookie.is_empty() {
            let cookie = self.cookie.to_owned();
            let tx_cookie = mpsc::Sender::clone(&tx);

            thread::spawn(move || {
                write_header_cookie(cookie, tx_cookie);
            });
        }

        // other header field-value pairs
        let header_set = self.header.to_owned();
        thread::spawn(move || {
            // tx_header has been moved in, no need to drop specifically
            write_headers(header_set, tx);
        });

        let mut header_misc = format!("Server: Rusty-Express/{}\r\n", VERSION);

        if !self.content_type.is_empty() {
            header_misc.push_str(&format!("Content-Type: {}\r\n", self.content_type));
        }

        if !self.header.contains_key("date") {
            let dt = Utc::now();
            header_misc.push_str(&format!("Date: {}\r\n", dt.format("%a, %e %b %Y %T GMT").to_string()));
        }

        if !self.header.contains_key("content-length") {
            if ignore_body || self.body.is_empty() {
                header_misc.push_str("Content-Length: 0\r\n");
            } else {
                header_misc.push_str(&format!("Content-Length: {}\r\n", self.body.len()));
            }
        }

        let mut header = Box::new(String::new());
        if let Ok(status) = rx_status.recv_timeout(Duration::from_millis(200)) {
            if !status.is_empty() {
                header.push_str(&status);
            } else {
                return Box::new(String::from("500 Internal Server Error\r\n\r\n"));
            }
        }

        for received in rx {
            if !received.is_empty() {
                header.push_str(&received);
            }
        }

        if !header_misc.is_empty() {
            header.push_str(&header_misc);
        }

        // write an empty line to end the header
        header.push_str("\r\n");
        header
    }
}

pub trait ResponseStates {
    fn to_close_connection(&self) -> bool;
    fn get_redirect_path(&self) -> String;
    fn get_header(&self, key: &str) -> Option<&String>;
    fn get_cookie(&self, key: &str) -> Option<&Cookie>;
    fn status_is_set(&self) -> bool;
    fn has_contents(&self) -> bool;
}

impl ResponseStates for Response {
    #[inline]
    fn to_close_connection(&self) -> bool {
        self.to_close
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

    fn status_is_set(&self) -> bool {
        match self.status {
            0 => false,
            _ => true,
        }
    }

    #[inline]
    fn has_contents(&self) -> bool {
        (!self.body.is_empty() && self.body.len() > 0)
    }
}

pub trait ResponseWriter {
    fn status(&mut self, status: u16);
    fn header(&mut self, field: &str, value: &str, replace: bool);
    fn send(&mut self, content: &str);
    fn send_file(&mut self, file_path: &str);
    fn set_cookie(&mut self, cookie: Cookie);
    fn set_cookies(&mut self, cookie: &[Cookie]);
    fn clear_cookies(&mut self);
    fn set_content_type(&mut self, content_type: String);
    fn check_and_update(&mut self, fallback: &HashMap<u16, String>);
    fn close_connection(&mut self, is_bad_request: bool);
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
            "content-type" => {
                self.content_type = value.to_owned();
            },
            _ => {
                set_header(&mut self.header, field.to_owned(), value.to_owned(), replace);
            },
        };
    }

    fn send(&mut self, content: &str) {
        if !content.is_empty() {
            self.body.push_str(content);
        }
    }

    fn send_file(&mut self, file_loc: &str) {
        if file_loc.is_empty() {
            println!("Undefined file path to retrieve data from...");
            return;
        }

        //TODO - 1: use meta path
        //TODO - 2: use 'view engine' to generate final markups --> use a different API for this

        let file_path = Path::new(file_loc);
        if !file_path.is_file() {
            // if doesn't exist or not a file, fail now
            println!("Can't locate requested file");
            self.status(404);
        } else {
            let status = read_from_file(&file_path, &mut self.body);
            if !self.status_is_set() { self.status(status); }

            if self.status == 200 && self.content_type.is_empty() {
                let mime_type =
                    if let Some(ext) = file_path.extension() {
                        let file_extension = ext.to_string_lossy().into_owned();
                        default_mime_type_with_ext(&file_extension[..])
                    } else {
                        String::from("text/plain")
                    };

                self.set_content_type(mime_type);
            }
        }
    }

    fn set_cookie(&mut self, cookie: Cookie) {
        if !cookie.is_valid() { return; }

        let key = cookie.get_cookie_key();
        if let Some(val) = self.cookie.get_mut(&key) {
            *val = cookie;
            return;
        }

        self.cookie.insert(key, cookie);
    }

    fn set_cookies(&mut self, cookies: &[Cookie]) {
        for cookie in cookies {
            self.set_cookie(cookie.to_owned());
        }
    }

    fn clear_cookies(&mut self) {
        self.cookie.clear();
    }

    fn set_content_type(&mut self, content_type: String) {
        if !content_type.is_empty() {
            self.content_type = content_type;
        }
    }

    fn check_and_update(&mut self, fallback: &HashMap<u16, String>) {
        //if contents have been provided, we're all good.
        if self.has_contents() { return; }

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

    fn close_connection(&mut self, is_bad_request: bool) {
        if is_bad_request {
            self.status(500);
        }

        self.to_close = true;
    }

    /// Can only redirect to internal path, no outsource path, sorry for the hackers (FYI, you can
    /// still hack the redirection link via Javascript)!
    fn redirect(&mut self, path: &str) {
        self.redirect = path.to_owned();
    }
}

pub trait ResponseStreamer {
    fn serialize_header(&self, buffer: &mut BufWriter<TcpStream>, ignore_body: bool);
    fn serialize_body(&self, buffer: &mut BufWriter<TcpStream>);
}

impl ResponseStreamer for Response {
    fn serialize_header(&self, buffer: &mut BufWriter<TcpStream>, ignore_body: bool) {
        if let Err(e) = buffer.write(self.resp_header(ignore_body).as_bytes()) {
            eprintln!("An error has taken place when writing the response header to the stream: {}", e);
        }
    }

    fn serialize_body(&self, buffer: &mut BufWriter<TcpStream>) {
        if self.has_contents() {
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

pub fn set_header<T>(header: &mut HashMap<String, T>, field: String, value: T, allow_override: bool) -> Option<T> {
    if field.is_empty() { return None; }

    let f = field.to_lowercase();
    if allow_override {
        //new field, insert
        header.insert(f, value)
    } else {
        //existing field, replace existing value or append depending on the parameter
        header.entry(f).or_insert(value);
        None
    }
}

fn get_default_page(buffer: &mut BufWriter<TcpStream>, status: u16) {
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
                println!("Unable to read file: {}", e);
                500
            },
            Ok(size) if size > 0 => {
                //things are truly ok now
                200
            },
            _ => {
                println!("File stream finds nothing...");
                404
            }
        };
    } else {
        println!("Unable to open requested file for path");
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
        "weba" => String::from("audio/webm"),
        "xhtml" => String::from("application/xhtml+xml"),
        "xul" => String::from("application/vnd.mozilla.xul+xml"),
        "7z" => String::from("application/x-7z-compressed"),
        "svg" => String::from("image/svg+xml"),
        "csh" | "sh" | "tar" | "wav" => {
            format!("application/x-{}", ext.to_owned())
        },
        "csv" | "html" | "htm" => {
            format!("text/{}", ext.to_owned())
        },
        "jpeg" | "jpg" | "gif" | "png" | "bmp" | "webp" | "tiff" | "tif" => {
            format!("image/{}", ext.to_owned())
        },
        "otf" | "ttf" | "woff" | "woff2" => {
            format!("font/{}", ext.to_owned())
        },
        "midi" | "mp3" | "aac" | "mid" | "oga"  => {
            format!("audio/{}", ext.to_owned())
        },
        "webm" | "mp4" | "ogg" | "mpeg" | "ogv" => {
            format!("video/{}", ext.to_owned())
        },
        "xml" | "pdf" | "json" | "ogx" | "rtf" | "zip" => {
            format!("application/{}", ext.to_owned())
        },
        _ => String::from("text/plain"),
    }
}

fn write_header_status(status: u16, has_contents: bool, tx: mpsc::Sender<String>) {
    let header: String;
    match status {
        404 | 500 => {
            header = get_status(status);
        },
        0 => {
            /* No status has been explicitly set, be smart here */
            if has_contents {
                header = get_status(200);
            } else {
                header = get_status(404);
            }
        },
        _ => {
            /* A status has been set explicitly, respect that here. */
            header = get_status(status);
        },
    }

    if let Err(e) = tx.send(header) {
        println!("Unable to write header status: {}", e);
    }
}

fn write_headers(header: HashMap<String, String>, tx: mpsc::Sender<String>) {
    let mut headers = String::new();

    for (field, value) in header.iter() {
        //special cases that shall be set using given methods
        let f = field.to_lowercase();
        if f.eq("content-type")
            || f.eq("date")
            || f.eq("content-length") {

            continue;
        }

        //otherwise, write to the header
        headers.push_str(&format!("{}: {}\r\n", field, value));
    }

    if let Err(e) = tx.send(headers) {
        println!("Unable to write main response header: {}", e);
    }
}

fn write_header_cookie(cookie: HashMap<String, Cookie>, tx: mpsc::Sender<String>) {
    println!("cookie parse");

    let mut set_cookie = String::new();
    for (_, cookie) in cookie.into_iter() {
        if cookie.is_valid() {
            set_cookie.push_str(&format!("Set-Cookie: {}\r\n", &cookie.to_string()));
        }
    }

    println!("cookie sent");

    if let Err(e) = tx.send(set_cookie) {
        println!("Unable to write response cookies: {}", e);
    }
}

