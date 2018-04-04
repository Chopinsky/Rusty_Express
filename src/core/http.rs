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
use core::config::{EngineContext, PageGenerator, ServerConfig, ViewEngine, ViewEngineParser};
use support::common::MapUpdates;
use support::debug;
use support::shared_pool;
use support::TaskType;

static FOUR_OH_FOUR: &'static str = include_str!("../default/404.html");
static FIVE_HUNDRED: &'static str = include_str!("../default/500.html");
static NEW_LINE: &'static str = "\r\n";
static VERSION: &'static str = "0.2.9";

pub struct Request {
    pub method: REST,
    pub uri: String,
    cookie: HashMap<String, String>,
    scheme: HashMap<String, Vec<String>>,
    fragment: String,
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
            fragment: String::new(),
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

    pub fn uri_fragment(&self) -> String {
        self.fragment.clone()
    }

    #[inline]
    pub fn param(&self, key: &str) -> Option<String> {
        match self.params.get(key) {
            Some(val) => Some(val.to_owned()),
            _ => None,
        }
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
    content_length: Option<String>,
    cookie: Arc<HashMap<String, Cookie>>,
    header: HashMap<String, String>,
    header_only: bool,
    body: Box<String>,
    body_tx: Option<mpsc::Sender<Box<String>>>,
    body_rx: Option<mpsc::Receiver<Box<String>>>,
    redirect: String,
}

impl Response {
    pub fn new() -> Self {
        Response {
            status: 0,
            keep_alive: false,
            content_type: String::new(),
            content_length: None,
            cookie: Arc::new(HashMap::new()),
            header: HashMap::new(),
            header_only: false,
            body: Box::new(String::new()),
            body_tx: None,
            body_rx: None,
            redirect: String::new(),
        }
    }

    pub fn new_with_default_header(default_header: HashMap<String, String>) -> Self {
        Response {
            status: 0,
            keep_alive: false,
            content_type: String::new(),
            content_length: None,
            cookie: Arc::new(HashMap::new()),
            header: default_header,
            header_only: false,
            body: Box::new(String::new()),
            body_tx: None,
            body_rx: None,
            redirect: String::new(),
        }
    }

    fn resp_header(&self) -> Box<String> {
        // Get cookie parser to its own thread
        let receiver: Option<mpsc::Receiver<String>> = match self.cookie.is_empty() {
            true => None,
            false => {
                let (tx, rx) = mpsc::channel();
                let cookie = Arc::clone(&self.cookie);

                shared_pool::run(move || {
                    write_header_cookie(cookie, tx);
                }, TaskType::Response);

                Some(rx)
            },
        };

        let mut header =
            Box::new(write_header_status(self.status, self.has_contents()));

        // other header field-value pairs
        write_headers(&self.header, &mut header);

        if !self.content_type.is_empty() {
            header.push_str(&format!("Content-Type: {}\r\n", self.content_type));
        }

        if let &Some(ref length) = &self.content_length {
            // explicit content length is set, use it here
            header.push_str(&format!("Content-Length: {}\r\n", length));
        } else if self.body_rx.is_none() {
            // Only generate content length header attribute if not using async and no content-length set explicitly
            if self.is_header_only() || self.body.is_empty() {
                header.push_str("Content-Length: 0\r\n");
            } else {
                header.push_str(&format!("Content-Length: {}\r\n", self.body.len()));
            }
        }

        if !self.header.contains_key("connection") {
            let connection = match self.keep_alive {
                true => "keep-alive",
                _ => "close",
            };

            header.push_str(&format!("Connection: {}\r\n", connection));
        }

        if let Some(rec) = receiver {
            if let Ok(content) = rec.recv_timeout(Duration::from_millis(64)) {
                if !content.is_empty() {
                    header.push_str(&content);
                }
            }
        }

        // if header only, we're done, write the new line as the EOF
        header.push_str(NEW_LINE);
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
    fn has_contents(&self) -> bool;
    fn is_header_only(&self) -> bool;
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
    fn has_contents(&self) -> bool {
        (self.is_header_only() || !self.body.is_empty() || self.body_rx.is_some())
    }

    #[inline]
    fn is_header_only(&self) -> bool {
        self.header_only
    }
}

pub trait ResponseWriter {
    fn status(&mut self, status: u16);
    fn header(&mut self, field: &str, value: &str, replace: bool);
    fn send(&mut self, content: &str);
    fn send_file(&mut self, file_path: &str) -> u16;
    fn send_file_async(&mut self, file_loc: &str);
    fn send_template(&mut self, file_path: &str, context: Box<EngineContext>) -> u16;
    fn set_cookie(&mut self, cookie: Cookie);
    fn set_cookies(&mut self, cookie: &[Cookie]);
    fn clear_cookies(&mut self);
    fn set_content_type(&mut self, content_type: &str);
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
            "content-length" => {
                let val = value.parse::<u64>();
                if let Ok(valid_len) = val {
                    self.content_length = Some(value.to_owned());
                } else {
                    panic!("Content length must be a valid string from u64, but provided with: {}", value);
                }
            },
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
        if self.is_header_only() { return; }

        if !content.is_empty() {
            self.body.push_str(content);
        }
    }

    /// Send a static file as part of the response to the client. Return the http
    /// header status that can be set directly to the response object using:
    ///
    ///     resp.status(<returned_status_value_from_this_api>);
    ///
    /// For example, if the file is read and parsed successfully, we will return 200;
    /// if we can't find the file, we will return 404; if there are errors when reading
    /// the file from its location, we will return 500.
    ///
    /// side effect: if the file is read and parsed successfully, we will set the
    /// content type based on file extension. You can always reset the value for
    /// this auto-generated content type response attribute.
    /// ...
    fn send_file(&mut self, file_loc: &str) -> u16 {
        //TODO - use meta path

        if self.is_header_only() { return 200; }

        if let Some(file_path) = get_file_path(file_loc) {
            let status = read_from_file(&file_path, &mut self.body);

            if status == 200 && self.content_type.is_empty() {
                let mime_type =
                    if let Some(ext) = file_path.extension() {
                        let file_extension = ext.to_string_lossy();
                        default_mime_type_with_ext(&file_extension)
                    } else {
                        String::from("text/plain")
                    };

                self.set_content_type(&mime_type[..]);
            }

            return status;
        }

        // if not getting the file path, then a 404
        404
    }

    fn send_file_async(&mut self, file_loc: &str) {
        if self.is_header_only() { return; }

        // lazy init the tx-rx pair.
        if self.body_tx.is_none() {
            let (tx, rx) = mpsc::channel();
            self.body_tx = Some(tx);
            self.body_rx = Some(rx);
        }

        if let &Some(ref tx) = &self.body_tx {
            let tx_clone = mpsc::Sender::clone(tx);
            let path = file_loc.to_owned();

            shared_pool::run(move || {
                read_from_file_async(path, tx_clone);
            }, TaskType::Body);
        }
    }

    fn send_template(&mut self, file_loc: &str, context: Box<EngineContext>) -> u16 {
        if self.is_header_only() { return 200; }

        //TODO - impl ServerConfig::template_parser
        //ServerConfig::template_parser();

        200
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

    fn keep_alive(&mut self, to_keep: bool) {
        self.keep_alive = to_keep;
    }

    /// Can only redirect to internal path, no outsource path, sorry for the hackers (FYI, you can
    /// still hack the redirection link via Javascript)!
    fn redirect(&mut self, path: &str) {
        self.redirect = path.to_owned();
    }
}

pub trait ResponseManager {
    fn header_only(&mut self, header_only: bool);
    fn validate_and_update(&mut self, fallback: &HashMap<u16, PageGenerator>);
    fn serialize_header(&self, buffer: &mut BufWriter<&TcpStream>);
    fn serialize_body(&self, buffer: &mut BufWriter<&TcpStream>);
}

impl ResponseManager for Response {
    #[inline]
    fn header_only(&mut self, header_only: bool) {
        self.header_only = header_only;
    }

    fn validate_and_update(&mut self, fallback: &HashMap<u16, PageGenerator>) {
        if self.body_tx.is_some() {
            // must drop the tx or we will hang indefinitely
            self.body_tx = None;
        }

        if self.status != 0 && (self.status < 200 || self.status == 204 || self.status == 304) {
            self.header_only(true);
        }

        // if contents have been provided, we're all good.
        if self.has_contents() { return; }

        // if not setting the header only and not having a body, it's a failure
        match self.status {
            0 | 404 => {
                if let Some(page_generator) = fallback.get(&404) {
                    //read_from_file(Path::new(file_path), &mut self.body);
                    self.body = Box::new(page_generator());
                } else {
                    self.body = Box::new(FOUR_OH_FOUR.to_owned());
                }
            },
            _ => {
                if let Some(page_generator) = fallback.get(&500) {
                    //read_from_file(Path::new(file_path), &mut self.body);
                    self.body = Box::new(page_generator());
                } else {
                    self.body = Box::new(FIVE_HUNDRED.to_owned());
                }
            },
        }
    }

    fn serialize_header(&self, buffer: &mut BufWriter<&TcpStream>) {
        if let Err(e) = buffer.write(self.resp_header().as_bytes()) {
            debug::print(&format!("An error has taken place when writing the response header to the stream: {}", e), 3);
        }
    }

    fn serialize_body(&self, buffer: &mut BufWriter<&TcpStream>) {
        if self.has_contents() {
            // content has been explicitly set, use them
            stream_response_body(self, buffer);
        } else {
            // this shouldn't happen, as we should have captured this in the check_and_update call
            stream_default_body(self.status, buffer);
        }
    }
}

fn stream_response_body(response: &Response, buffer: &mut BufWriter<&TcpStream>) {
    if !response.body.is_empty() {
        // the content length should have been set in the header, see function resp_header
        if let Err(e) = buffer.write(response.body.as_bytes()) {
            debug::print(
                &format!("An error has taken place when writing the response header to the stream: {}", e),
                1);
        }
    }

    if let Some(ref rx) = response.body_rx {
        for received in rx {
            if !received.is_empty() {
                if let Err(e) = buffer.write(received.as_bytes()) {
                    debug::print(
                        &format!("An error has taken place when writing the response header to the stream: {}", e),
                        1);
                }
            }
        }
    }
}

fn stream_default_body(status: u16, buffer: &mut BufWriter<&TcpStream>) {
    match status {
        //explicit error status
        0 | 404 => write_default_page(400, buffer),
        500 => write_default_page(500, buffer),
        _ => { /* Nothing */ },
    };
}

fn write_default_page(status: u16, buffer: &mut BufWriter<&TcpStream>) {
    match status {
        500 => {
            /* return default 500 page */
            if let Err(e) = buffer.write(FIVE_HUNDRED.as_bytes()) {
                debug::print(
                    &format!("An error has taken place when writing the response body to the stream: {}", e),
                    1);
            }
        },
        _ => {
            /* return default/override 404 page */
            if let Err(e) = buffer.write(FOUR_OH_FOUR.as_bytes()) {
                debug::print(
                    &format!("An error has taken place when writing the response body to the stream: {}", e),
                    1);
            }
        },
    }
}

fn get_file_path(path: &str) -> Option<&Path> {
    if path.is_empty() {
        debug::print("Undefined file path to retrieve data from...", 1);
        return None;
    }

    let file_path = Path::new(path);
    if !file_path.is_file() {
        debug::print("Can't locate requested file", 1);
        return None;
    }

    Some(file_path)
}

fn read_from_file(file_path: &Path, buf: &mut Box<String>) -> u16 {
    // try open the file
    if let Ok(file) = File::open(file_path) {
        let mut buf_reader = BufReader::new(file);
        return match buf_reader.read_to_string(buf) {
            Err(e) => {
                debug::print(&format!("Unable to read file: {}", e), 1);
                500
            },
            Ok(_) => {
                //things are truly ok now
                200
            },
        };
    } else {
        debug::print("Unable to open requested file for path", 1);
        404
    }
}

fn read_from_file_async(file_loc: String, tx: mpsc::Sender<Box<String>>) {
    if let Some(file_path) = get_file_path(&file_loc[..]) {
        let mut buf: Box<String> = Box::new(String::new());
        match read_from_file(file_path, &mut buf) {
            404 | 500 if buf.len() > 0 => { buf.clear(); },
            _ => { /* Nothing to do here */ },
        }

        if let Err(e) = tx.send(buf) {
            debug::print(&format!("Unable to write the file to the stream: {}", e), 1);
        }
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

    if !header.contains_key("date") {
        let dt = Utc::now();
        final_header.push_str(&format!("Date: {}\r\n", dt.format("%a, %e %b %Y %T GMT").to_string()));
    }

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
        debug::print(&format!("Unable to write response cookies: {}", e), 1);
    });
}

