#![allow(dead_code)]

use std::collections;
use std::fs::File;
use std::io::{prelude::*, BufReader, BufWriter};
use std::mem;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::ptr;
use std::str;
use std::thread;
use std::time::Duration;

use crate::channel::{self, Receiver, Sender, TryRecvError};
use crate::chrono::prelude::*;
use crate::core::syncstore::{Reusable, StaticStore, SyncPool, TOTAL_ELEM_COUNT};
use crate::core::{
    config::{ConnMetadata, EngineContext, ServerConfig, ViewEngineParser},
    cookie::*,
    router::REST,
    stream::Stream,
};
use crate::hashbrown::{hash_map::Iter, HashMap};
use crate::support::{common::*, debug, debug::InfoLevel, shared_pool, TaskType};
use std::collections::hash_map::RandomState;

const FOUR_OH_FOUR: &str = include_str!("../default/404.html");
const FOUR_OH_ONE: &str = include_str!("../default/401.html");
const FIVE_HUNDRED: &str = include_str!("../default/500.html");
const VERSION: &str = env!("CARGO_PKG_VERSION");

const RESP_TIMEOUT: Duration = Duration::from_millis(64);
const LONG_CONN_TIMEOUT: Duration = Duration::from_secs(8);
const HEADER_END: [u8; 2] = [13, 10];

type BodyChan = (
    Option<Sender<(Vec<u8>, u16)>>,
    Option<Receiver<(Vec<u8>, u16)>>,
);
type NotifyChan = Option<(Sender<String>, Receiver<String>)>;

static mut REQ_POOL: StaticStore<SyncPool<Request>> = StaticStore::init();
static mut RESP_POOL: StaticStore<SyncPool<Response>> = StaticStore::init();
static mut POOL_CHAN: StaticStore<(channel::Sender<()>, channel::Receiver<()>)> =
    StaticStore::init();

//TODO: pub http version?

#[derive(PartialOrd, PartialEq)]
enum KeepAliveStatus {
    NotSet,
    TlsConn,
    Forbidden,
    KeepAlive,
}

impl Default for KeepAliveStatus {
    fn default() -> Self {
        KeepAliveStatus::NotSet
    }
}

#[derive(Default)]
pub struct Request {
    pub method: REST,
    pub uri: String,
    params: HashMap<String, String>,
    query: HashMap<String, Vec<String>>,
    header: HashMap<String, String>,
    cookie: HashMap<String, String>,
    fragment: String,
    host: String,
    body: String,
    client_info: Option<SocketAddr>,
}

impl Request {
    #[inline]
    pub(crate) fn new() -> Self {
        Default::default()
    }

    pub fn header(&self, field: &str) -> Option<String> {
        if field.is_empty() {
            return None;
        }

        if self.header.is_empty() {
            return None;
        }

        match self.header.get(&field[..]) {
            Some(value) => Some(value.to_owned()),
            None => None,
        }
    }

    pub fn keep_alive(&self) -> bool {
        match self.header("connection") {
            None => false,
            Some(val) => &val != "keep-alive",
        }
    }

    pub fn cookie(&self, key: &str) -> Option<String> {
        if key.is_empty() {
            return None;
        }

        if self.cookie.is_empty() {
            return None;
        }

        match self.cookie.get(&key[..]) {
            Some(value) => Some(value.to_owned()),
            None => None,
        }
    }

    #[inline]
    pub fn cookie_iter(&self) -> Iter<String, String> {
        self.cookie.iter()
    }

    pub fn query(&self, field: &str) -> Option<Vec<String>> {
        if field.is_empty() {
            return None;
        }

        if self.query.is_empty() {
            return None;
        }

        match self.query.get(&field[..]) {
            Some(value) => Some(value.to_owned()),
            None => None,
        }
    }

    pub fn uri_fragment(&self) -> String {
        self.fragment.clone()
    }

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

    #[inline]
    pub fn client_info(&self) -> Option<SocketAddr> {
        self.client_info
    }

    #[inline]
    pub fn host_info(&self) -> String {
        self.host.clone()
    }

    #[must_use]
    pub fn form_data(&self) -> collections::HashMap<String, String> {
        let mut data = collections::HashMap::new();

        self.body.split('&').for_each(|seg: &str| {
            if let Some(pos) = seg.find('=') {
                data.insert(String::from(&seg[..pos]), String::from(&seg[pos + 1..]));
            }
        });

        data
    }

    pub fn json(&self) -> String {
        let mut source = HashMap::new();

        source.insert(String::from("method"), self.method.to_string());
        source.insert(String::from("uri"), self.uri.to_owned());

        if !self.params.is_empty() {
            source.insert(String::from("uri_params"), json_stringify(&self.params));
        }

        if !self.query.is_empty() {
            source.insert(String::from("uri_querys"), json_flat_stringify(&self.query));
        }

        if !self.fragment.is_empty() {
            source.insert(String::from("uri_fragment"), self.fragment.to_owned());
        }

        if !self.body.is_empty() {
            source.insert(String::from("body"), self.body.to_owned());
        }

        if !self.header.is_empty() {
            source.insert(String::from("headers"), json_stringify(&self.header));
        }

        if !self.cookie.is_empty() {
            source.insert(String::from("cookies"), json_stringify(&self.cookie));
        }

        if !self.host.is_empty() {
            source.insert(String::from("host"), self.host.to_owned());
        }

        if let Some(addr) = self.client_info {
            source.insert(String::from("socket_address"), addr.to_string());
        }

        json_stringify(&source)
    }

    pub(crate) fn set_headers(&mut self, header: HashMap<String, String>) {
        self.header = header;

        if let Some(host_name) = self.header.get(&String::from("host")) {
            self.host = host_name.to_owned();
        }
    }

    pub(crate) fn set_cookies(&mut self, cookie: HashMap<String, String>) {
        self.cookie = cookie;
    }

    pub(crate) fn set_body(&mut self, body: String) {
        self.body = body;
    }
}

impl Reusable for Request {
    fn obtain() -> Box<Self> {
        match unsafe { REQ_POOL.as_mut() } {
            Ok(pool) => pool.get(),
            Err(_) => Default::default(),
        }
    }

    fn release(mut self: Box<Self>) {
        self.reset(false);

        if let Ok(pool) = unsafe { REQ_POOL.as_mut() } {
            pool.put(self);
        }
    }

    fn reset(&mut self, hard: bool) {
        self.method = REST::GET;

        if !hard {
            unsafe {
                self.uri.as_mut_vec().set_len(0);
                self.fragment.as_mut_vec().set_len(0);
                self.host.as_mut_vec().set_len(0);
                self.body.as_mut_vec().set_len(0);
            }
        } else {
            self.uri.clear();
            self.fragment.clear();
            self.host.clear();
            self.body.clear();
        }

        self.params.clear();
        self.query.clear();
        self.header.clear();
        self.cookie.clear();

        if self.client_info.is_some() {
            self.client_info.take();
        }
    }
}

pub trait RequestWriter {
    fn write_header(&mut self, key: &str, val: &str, allow_override: bool);
    fn write_query(&mut self, key: &str, val: Vec<String>, allow_override: bool);
    fn create_query(&mut self, query: HashMap<String, Vec<String>>);
    fn set_cookie(&mut self, key: &str, val: &str, allow_override: bool);
    fn create_cookie(&mut self, cookie: HashMap<String, String>);
    fn set_param(&mut self, key: &str, val: &str);
    fn create_param(&mut self, params: HashMap<String, String>);
    fn set_fragment(&mut self, fragment: String);
    fn set_host(&mut self, host: String);
    fn set_client(&mut self, addr: SocketAddr);
    fn extend_body(&mut self, content: &str);
}

impl RequestWriter for Request {
    fn write_header(&mut self, key: &str, val: &str, allow_override: bool) {
        self.header.add(key, val.to_owned(), allow_override, false);
    }

    fn write_query(&mut self, key: &str, val: Vec<String>, allow_override: bool) {
        self.query.add(key, val.to_owned(), allow_override, false);
    }

    fn create_query(&mut self, query: HashMap<String, Vec<String>>) {
        self.query = query;
    }

    fn set_cookie(&mut self, key: &str, val: &str, allow_override: bool) {
        self.cookie.add(key, val.to_owned(), allow_override, true);
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

    #[inline]
    fn set_fragment(&mut self, fragment: String) {
        self.fragment = fragment;
    }

    fn set_host(&mut self, host: String) {
        self.host = host;
    }

    #[inline]
    fn set_client(&mut self, addr: SocketAddr) {
        self.client_info = Some(addr)
    }

    fn extend_body(&mut self, content: &str) {
        self.body.push_str(content);
    }
}

#[derive(Default)]
pub struct Response {
    status: u16,
    keep_alive: KeepAliveStatus,
    content_type: String,
    content_length: Option<String>,
    header: HashMap<String, String>,
    cookie: HashMap<String, Cookie>,
    header_only: bool,
    redirect: String,
    body: Vec<u8>,
    body_chan: BodyChan,
    notifier: NotifyChan,
    subscriber: NotifyChan,
}

impl Response {
    pub(crate) fn new() -> Self {
        Default::default()
    }

    pub(crate) fn default_header(&mut self, header: HashMap<String, String>) {
        self.header = header;
    }

    pub(crate) fn redirect_handling(&mut self) {
        // if a redirect response, set up as so.
        let mut redirect = self.get_redirect_path();

        if !redirect.is_empty() {
            if !redirect.starts_with('/') {
                redirect.insert(0, '/');
            }

            self.header("Location", &redirect, true);
            self.status(301);
        }
    }

    fn write_resp_header(&mut self, buffer: &mut BufWriter<&mut Stream>) {
        // Get cookie parser to its own thread
        let receiver: Option<Receiver<Vec<u8>>> = if self.cookie.is_empty() {
            None
        } else {
            let (tx, rx) = channel::bounded(1);
            let cookie = mem::replace(&mut self.cookie, HashMap::new());

            shared_pool::run(
                move || {
                    write_header_cookie(cookie, tx);
                },
                TaskType::Response,
            );

            Some(rx)
        };

        // get the initial header line
        let mut header = write_header_status(self.status, self.has_contents());

        // other header field-value pairs
        write_headers(&self.header, &mut header, self.to_keep_alive());

        // write to the buffer first
        buffer.write(&header.swap_reset()).unwrap_or_default();

        // write the remainder headers
        if !self.content_type.is_empty() {
            header.reserve(16 + self.content_type.len());
            header.extend_from_slice(b"Content-Type: ");
            header.extend_from_slice(self.content_type.as_bytes());
            header.append_line_break();
        }

        if let Some(length) = self.content_length.as_ref() {
            // explicit content length is set, use it here
            header.reserve(18 + length.len());
            header.extend_from_slice(b"Content-Length: ");
            header.extend_from_slice(length.as_bytes());
            header.append_line_break();
        } else {
            // Only generate content length header attribute if not using async and no content-length set explicitly
            if self.is_header_only() || self.body.is_empty() {
                header.reserve(19);
                header.extend_from_slice(b"Content-Length: 0\r\n");
            } else {
                let size = self.body.len().to_string();

                header.reserve(18 + size.len());
                header.extend_from_slice(b"Content-Length: ");
                header.extend_from_slice(size.as_bytes());
                header.append_line_break();
            }
        }

        if !self.header.contains_key("connection") {
            let (connection, count) = if self.to_keep_alive() {
                ("keep-alive", 10)
            } else {
                ("close", 5)
            };

            header.reserve(14 + count);
            header.extend_from_slice(b"Connection: ");
            header.extend_from_slice(connection.as_bytes());
            header.append_line_break();
        }

        // we're pretty much done, write the content to the underlying buffer.
        buffer.write(&header).unwrap_or_default();

        if let Some(rx) = receiver {
            if let Ok(content) = rx.recv_timeout(RESP_TIMEOUT) {
                if !content.is_empty() && buffer.write(&content).is_err() {
                    debug::print("Failed to send cookie headers", InfoLevel::Warning);
                }
            }
        }
    }

    fn set_ext_mime_header(&mut self, path: &PathBuf) {
        let mime_type = if let Some(ext) = path.extension() {
            let file_extension = ext.to_string_lossy();
            default_mime_type_with_ext(&file_extension)
        } else {
            String::from("text/plain")
        };

        self.set_content_type(&mime_type);
    }
}

impl Reusable for Response {
    fn obtain() -> Box<Self> {
        match unsafe { RESP_POOL.as_mut() } {
            Ok(pool) => pool.get(),
            Err(_) => Default::default(),
        }
    }

    fn release(mut self: Box<Self>) {
        self.reset(false);

        if let Ok(pool) = unsafe { RESP_POOL.as_mut() } {
            pool.put(self);
        }
    }

    fn reset(&mut self, hard: bool) {
        self.status = 0;
        self.keep_alive = KeepAliveStatus::NotSet;

        if !hard {
            unsafe {
                self.content_type.as_mut_vec().set_len(0);
                self.redirect.as_mut_vec().set_len(0);
                self.body.set_len(0);
            }
        } else {
            self.content_type.clear();
            self.redirect.clear();
            self.body.clear();
        }

        if self.content_length.is_some() {
            self.content_length.take();
        }

        self.header_only = false;
        self.header.clear();
        self.cookie.clear();
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
    fn get_channels(&mut self) -> Result<(Sender<String>, Receiver<String>), &'static str>;
}

impl ResponseStates for Response {
    #[inline]
    fn to_keep_alive(&self) -> bool {
        self.keep_alive == KeepAliveStatus::KeepAlive
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
        (self.is_header_only() || !self.body.is_empty() || self.body_chan.0.is_some())
    }

    #[inline]
    fn is_header_only(&self) -> bool {
        self.header_only
    }

    /// get_channels will create the channels for communicating between the chunk generator threads and
    /// the main stream. Listen to the receiver for any client communications, and use the sender to
    /// send any ensuing responses.
    fn get_channels(&mut self) -> Result<(Sender<String>, Receiver<String>), &'static str> {
        if self.notifier.is_none() {
            self.notifier = Some(channel::bounded(64));
        }

        if self.subscriber.is_none() {
            self.subscriber = Some(channel::bounded(64));
        }

        if let Some(notifier) = self.notifier.as_ref() {
            if let Some(sub) = self.subscriber.as_ref() {
                return Ok((notifier.0.clone(), sub.1.clone()));
            }
        }

        debug::print("Unable to create channels", InfoLevel::Warning);
        Err("Unable to create channels")
    }
}

pub trait ResponseWriter {
    fn status(&mut self, status: u16);
    fn header(&mut self, field: &str, value: &str, allow_replace: bool);
    fn set_header(&mut self, field: &str, value: &str);
    fn with_headers(&mut self, header: HashMap<String, String>);
    fn send(&mut self, content: &str);
    fn send_async(&mut self, f: fn() -> (Option<u16>, String));
    fn send_file(&mut self, file_path: &str) -> u16;
    fn send_file_from_path(&mut self, path: PathBuf) -> u16;
    fn send_file_async(&mut self, file_loc: &str);
    fn send_file_from_path_async(&mut self, path: PathBuf);
    fn send_template<T: EngineContext + Send + Sync + 'static>(
        &mut self,
        file_path: &str,
        context: Box<T>,
    ) -> u16;
    fn set_cookie(&mut self, cookie: Cookie);
    fn set_cookies(&mut self, cookie: &[Cookie]);
    fn clear_cookies(&mut self);
    fn can_keep_alive(&mut self, can_keep_alive: bool);
    fn keep_alive(&mut self, to_keep: bool);
    fn set_content_type(&mut self, content_type: &str);
    fn redirect(&mut self, path: &str);
}

impl ResponseWriter for Response {
    /// Set the status code of the response. This will always override any existing values set to the
    /// response already (by self, by someone else, or by middlewear). Note that we will enforce the
    /// code to be written to the header, if it's a number not recognized by the server, we will default
    /// to use status code 200 OK for the response.
    fn status(&mut self, status: u16) {
        self.status = match status {
            100..=101 => status,
            200..=206 => status,
            300..=308 if status != 307 && status != 308 => status,
            400..=417 if status != 402 => status,
            426 | 428 | 429 | 431 | 451 => status,
            500..=505 | 511 => status,
            _ => 0,
        };
    }

    /// `header` is the base API to set 1 field in the header, note that the value shall represent
    /// the entire content to be put in the response's http header.
    ///
    /// By default, the field-value pair will be set to the response header, and the caller can
    /// control if this operation can override any existing pairs if they've been set prior to the
    /// function call.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rusty_express::prelude::*;
    ///
    /// pub fn simple_handler(req: &Box<Request>, resp: &mut Box<Response>) {
    ///     resp.header("Content-Type", "application/javascript", true);
    ///     assert!(resp.get_header("Content-Type").unwrap(), &String::from("application/javascript"));
    /// }
    /// ```
    fn header(&mut self, field: &str, value: &str, allow_replace: bool) {
        if field.is_empty() || value.is_empty() {
            return;
        }

        let key = field.to_lowercase();
        match &key[..] {
            "content-type" => {
                if self.content_type.is_empty() || allow_replace {
                    // only set the content type if the first time, or allow replacing the existing value
                    self.content_type = value.to_owned();
                }
            }
            "content-length" => {
                if self.content_length.is_some() && !allow_replace {
                    // if the content length has been set and we won't allow override, we're done
                    return;
                }

                if let Ok(valid_len) = value.parse::<u64>() {
                    self.content_length = Some(value.to_string());
                } else {
                    panic!(
                        "Content length must be a valid string from u64, but provided with: {}",
                        value
                    );
                }
            }
            "connection" => {
                if self.to_keep_alive() && !allow_replace {
                    // if already toggled to keep alive and don't allow override, we're done
                    return;
                }

                match &value.to_lowercase()[..] {
                    "keep-alive" if self.keep_alive == KeepAliveStatus::NotSet => {
                        self.keep_alive = KeepAliveStatus::KeepAlive
                    }
                    "tls" => self.keep_alive = KeepAliveStatus::TlsConn,
                    "forbidden" => self.keep_alive = KeepAliveStatus::Forbidden,
                    _ => { /* Otherwise, don't update the keep_alive field */ }
                }
            }
            _ => {
                self.header
                    .add(field, value.to_owned(), allow_replace, false);
            }
        };
    }

    /// `set_header` is a sugar to the `header` API, and it's created to simplify the majority of the
    /// use cases for setting the response header.
    ///
    /// By default, the field-value pair will be set to the response header, and override any existing
    /// pairs if they've been set prior to the API call.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rusty_express::prelude::*;
    ///
    /// pub fn simple_handler(req: &Box<Request>, resp: &mut Box<Response>) {
    ///     resp.set_header("Content-Type", "application/javascript");
    ///     assert!(resp.get_header("Content-Type").unwrap(), &String::from("application/javascript"));
    /// }
    /// ```
    #[inline]
    fn set_header(&mut self, field: &str, value: &str) {
        self.header(field, value, true);
    }

    /// Define the response headers with pre-defined headers, such that the headers can be reused if
    /// controlled in the server level or defined in the contentext.
    ///
    /// # Examples
    ///
    /// Define a header with another method:
    ///
    /// ```rust
    /// use rusty_express::prelude::*;
    /// use std::collections::HashMap;
    ///
    /// pub fn simple_handler(req: &Box<Request>, resp: &mut Box<Response>) {
    ///    // process request to extract info for generating the response body.
    ///    let header = header_producer(req);
    ///    resp.with_headers(header);
    ///
    ///    // Send the content back
    ///    resp.send("{ id: 1, name: 'John Doe', age: NaN }");
    /// }
    /// ```
    fn with_headers(&mut self, mut header: HashMap<String, String>) {
        if let Some(val) = header.remove("content-type") {
            self.content_type = val;
        }

        if let Some(val) = header.remove("content-length") {
            self.content_length = val.parse::<u64>().ok().map(|length| length.to_string());
        }

        if let Some(val) = header.remove("connection") {
            match &val.to_lowercase()[..] {
                "keep-alive" if self.keep_alive == KeepAliveStatus::NotSet => {
                    self.keep_alive = KeepAliveStatus::KeepAlive
                }
                "tls" => self.keep_alive = KeepAliveStatus::TlsConn,
                "forbidden" => self.keep_alive = KeepAliveStatus::Forbidden,
                _ => { /* Otherwise, don't update the keep_alive field */ }
            }
        }

        self.header = header;
    }

    /// The main API for setting the body content of the response.
    ///
    /// # Examples
    ///
    /// Sending responses
    ///
    /// ```rust
    /// use rusty_express::prelude::*;
    /// use std::collections::HashMap;
    /// use std::hash::BuildHasherDefault;
    ///
    /// pub fn simple_handler(req: &Box<Request>, resp: &mut Box<Response>) {
    ///    // process request to extract info for generating the response body.
    ///    let meta_data: SomeMetadata = some_producer(req);
    ///
    ///    // the `generate_response_body` will create all contents to be returned
    ///    resp.send(&generate_response_body(meta_data));
    /// }
    /// ```
    fn send(&mut self, content: &str) {
        if self.is_header_only() {
            return;
        }

        if !content.is_empty() {
            self.body.reserve(content.len());
            self.body.extend_from_slice(content.as_bytes());
        }
    }

    /// Send the response body in async mode. This means the closure or function supplied as the 1st
    /// parameter will be executed in parallel.
    ///
    /// # Examples
    ///
    /// Computing response body in async mode:
    ///
    /// ```rust
    /// use rusty_express::prelude::*;
    /// use std::collections::HashMap;
    /// use std::hash::BuildHasherDefault;
    ///
    /// pub fn simple_handler(req: &Box<Request>, resp: &mut Box<Response>) {
    ///    // process request to extract info for generating the response body.
    ///    let meta_data: SomeMetadata = some_producer(req);
    ///
    ///    // the heavy method will *NOT* block in this case. The closure's return value
    ///    // shall be a tuple: 1) the 1st param shall be a `Option<u16>` for any special
    ///    // status code for the response, and if it's `None`, we will use status code 200
    ///    // as the default value; 2) the 2nd param shall be a `String` that will comprise
    ///    // the response body. Note that the response generated by this method will be appended
    ///    // to any existing value set to the response body previously.
    ///    resp.send_async(|| (None, heavy_method(meta_data)));
    ///
    ///    // The above method call is equivalent to the line below:
    ///    // resp.send_async(|| (Some(200), heavy_method(meta_data)));
    /// }
    /// ```
    fn send_async(&mut self, f: fn() -> (Option<u16>, String)) {
        // if header only, quit
        if self.is_header_only() {
            return;
        }

        // lazy init the tx-rx pair.
        if self.body_chan.0.is_none() {
            let (tx, rx) = channel::bounded(4);
            self.body_chan = (Some(tx), Some(rx));
        }

        if let Some(tx) = self.body_chan.0.as_ref() {
            let tx_clone = tx.clone();

            shared_pool::run(
                move || {
                    let (status, content) = f();
                    tx_clone
                        .send((Vec::from(content), status.unwrap_or(200)))
                        .unwrap_or_default();
                },
                TaskType::Response,
            );
        }
    }

    /// Send a static file as part of the response to the client. Return the http
    /// header status that can be set directly to the response object using:
    ///
    /// # Examples
    ///
    /// Computing response body from a file location:
    ///
    /// ```rust
    /// use rusty_express::prelude::*;
    /// use std::collections::HashMap;
    /// use std::hash::BuildHasherDefault;
    ///
    /// pub fn simple_handler(req: &Box<Request>, resp: &mut Box<Response>) {
    ///     // process request to extract info for generating the response body.
    ///     let file_loc: String = get_file_loc(req);
    ///
    ///     let status = resp.send_file(&file_loc);
    ///     resp.status(status);
    /// }
    /// ```
    ///
    /// For example, if the file is read and parsed successfully, we will return 200;
    /// if we can't find the file, we will return 404; if there are errors when reading
    /// the file from its location, we will return 500.
    ///
    /// Note the side effect: if the file is read and parsed successfully, we will set the
    /// content type based on file extension. You can always reset the value for
    /// this auto-generated content type response attribute.
    fn send_file(&mut self, file_loc: &str) -> u16 {
        if self.is_header_only() {
            return 200;
        }

        if let Some(file_path) = get_file_path(file_loc) {
            return self.send_file_from_path(file_path);
        }

        // if not getting the file path, then a 404
        404
    }

    fn send_file_from_path(&mut self, path: PathBuf) -> u16 {
        if self.is_header_only() {
            return 200;
        }

        let status = open_file(&path, &mut self.body);

        if status != 200 && status != 0 {
            // if not opening the file correctly, reset the body for error page
            unsafe {
                self.body.set_len(0);
            }
        } else if status == 200 && self.content_type.is_empty() {
            // if read the file good and not set the mime yet, set the mime
            self.set_ext_mime_header(&path);
        }

        status
    }

    fn send_file_async(&mut self, file_loc: &str) {
        if let Some(path) = get_file_path(file_loc) {
            self.send_file_from_path_async(path);
        }
    }

    fn send_file_from_path_async(&mut self, path: PathBuf) {
        // if header only, quit
        if self.is_header_only() {
            return;
        }

        // lazy init the tx-rx pair.
        if self.body_chan.0.is_none() {
            let (tx, rx) = channel::bounded(4);
            self.body_chan = (Some(tx), Some(rx));
        }

        // set header's mime extension field
        self.set_ext_mime_header(&path);

        // actually load the file to the response body
        if let Some(chan) = self.body_chan.0.as_ref() {
            open_file_async(path, chan.clone());
        }
    }

    fn send_template<T: EngineContext + Send + Sync + 'static>(
        &mut self,
        file_path: &str,
        context: Box<T>,
    ) -> u16 {
        if self.is_header_only() {
            return 200;
        }
        if file_path.is_empty() {
            return 404;
        }

        if let Some(path) = get_file_path(file_path) {
            if !path.is_file() {
                return 404;
            }

            let mut ext = String::new();
            if let Some(os_ext) = path.extension() {
                if let Some(str_ext) = os_ext.to_str() {
                    ext = str_ext.to_owned();
                }
            }

            if ext.is_empty() {
                return 404;
            }

            let mut content = Vec::new();
            open_file(&path, &mut content);

            // Now render the conent with the engine
            let (status, final_content) = ServerConfig::template_parser(&ext[..], content, context);

            if status == 0 || status == 200 {
                self.body = final_content;
                if self.content_type.is_empty() {
                    // if read the file good and not set the mime yet, set the mime
                    self.set_ext_mime_header(&path);
                }
            }

            status
        } else {
            404
        }
    }

    fn set_cookie(&mut self, cookie: Cookie) {
        if !cookie.is_valid() {
            return;
        }

        let key = cookie.get_cookie_key();
        self.cookie.insert(key, cookie);
    }

    fn set_cookies(&mut self, cookies: &[Cookie]) {
        for cookie in cookies.iter() {
            if !cookie.is_valid() {
                continue;
            }

            let key = cookie.get_cookie_key();
            self.cookie.insert(key, cookie.clone());
        }
    }

    fn clear_cookies(&mut self) {
        self.cookie.clear();
    }

    #[inline]
    fn can_keep_alive(&mut self, can_keep_alive: bool) {
        self.keep_alive = if !can_keep_alive {
            KeepAliveStatus::Forbidden
        } else {
            KeepAliveStatus::NotSet
        }
    }

    fn keep_alive(&mut self, to_keep: bool) {
        if self.keep_alive == KeepAliveStatus::TlsConn
            || self.keep_alive == KeepAliveStatus::Forbidden
        {
            return;
        }

        self.keep_alive = if to_keep {
            KeepAliveStatus::KeepAlive
        } else {
            KeepAliveStatus::NotSet
        };
    }

    fn set_content_type(&mut self, content_type: &str) {
        if !content_type.is_empty() {
            self.content_type = content_type.to_owned();
        }
    }

    /// Can only redirect to internal path, no outsource path, sorry for the hackers (FYI, you can
    /// still hack the redirection link via Javascript)!
    fn redirect(&mut self, path: &str) {
        self.redirect = path.to_owned();
    }
}

pub(crate) trait ResponseManager {
    fn header_only(&mut self, header_only: bool);
    fn validate_and_update(&mut self);
    fn write_header(&mut self, buffer: &mut BufWriter<&mut Stream>) -> bool;
    fn write_body(&self, buffer: &mut BufWriter<&mut Stream>) -> bool;
    fn keep_long_conn(&mut self, clone: Stream, buffer: &mut BufWriter<&mut Stream>);
}

impl ResponseManager for Response {
    #[inline]
    fn header_only(&mut self, header_only: bool) {
        self.header_only = header_only;
    }

    fn validate_and_update(&mut self) {
        if self.status != 0 && (self.status < 200 || self.status == 204 || self.status == 304) {
            self.header_only(true);
        }

        if !self.is_header_only() && self.body_chan.1.is_some() {
            // manual drop the transmission channel so we won't hang forever. this only drops the origin
            // channel, all clones (which must have been created before reaching this point) can still
            // be valid at this point, and the rx loop will either enter or break after the last one
            // is dropped from the async tasks.
            drop(self.body_chan.0.take());

            // try to receive the async bodies
            if let Some(chan) = self.body_chan.1.take() {
                for received in chan {
                    if received.1 == 200 {
                        // read the content
                        if !received.0.is_empty() {
                            self.body.reserve(received.0.len());
                            self.body.extend_from_slice(&received.0);
                        }
                    } else {
                        // faulty, clear the content
                        self.status = 500;
                        self.body.clear();
                        break;
                    }
                }
            }
        }

        // if contents have been provided, we're all good.
        if self.has_contents() {
            return;
        }

        // if not setting the header only and not having a body, it's a failure
        match self.status {
            0 | 404 => {
                if let Some(page_generator) = ConnMetadata::get_status_pages(404) {
                    self.body = page_generator().into_bytes();
                } else {
                    self.body = Vec::from(FOUR_OH_FOUR.as_bytes());
                }
            }
            401 => {
                if let Some(page_generator) = ConnMetadata::get_status_pages(401) {
                    self.body = page_generator().into_bytes();
                } else {
                    self.body = Vec::from(FOUR_OH_ONE.as_bytes());
                }
            }
            _ => {
                if let Some(page_generator) = ConnMetadata::get_status_pages(500) {
                    self.body = page_generator().into_bytes();
                } else {
                    self.body = Vec::from(FIVE_HUNDRED.as_bytes());
                }
            }
        }
    }

    fn write_header(&mut self, buffer: &mut BufWriter<&mut Stream>) -> bool {
        // write the headers
        self.write_resp_header(buffer);

        // write_to_buff(buffer, self.resp_header().as_bytes());

        // Blank line to indicate the end of the response header
        write_to_buff(buffer, &HEADER_END);

        // flush what we got so far
        buffer.flush().is_ok()
    }

    fn write_body(&self, buffer: &mut BufWriter<&mut Stream>) -> bool {
        if self.has_contents() {
            // the content length should have been set in the header, see function resp_header
            write_to_buff(buffer, &self.body);
        } else {
            // this shouldn't happen, as we should have captured this in the check_and_update call
            stream_default_body(self.status, buffer);
        }

        buffer.flush().is_ok()
    }

    fn keep_long_conn(&mut self, stream_clone: Stream, buffer: &mut BufWriter<&mut Stream>) {
        if self.has_contents() {
            // the content length should have been set in the header, see function resp_header
            stream_trunk(&self.body, buffer);
        }

        // set read time-out to 16 seconds
        if let Err(e) = stream_clone.set_read_timeout(Some(LONG_CONN_TIMEOUT)) {
            debug::print(
                &format!(
                    "Failed to establish a reading channel on a keep-alive stream: {}",
                    e
                ),
                InfoLevel::Warning,
            );
            return;
        }

        if let Some(sub) = self.subscriber.as_ref() {
            // spawn a new thread to listen to the read stream for any new communications
            broadcast_new_communications(sub.0.clone(), stream_clone);
        }

        if let Some(ref notifier) = self.notifier {
            // listen to any replies from the server routes
            while let Ok(mut message) = notifier.1.recv_timeout(LONG_CONN_TIMEOUT) {
                stream_trunk(unsafe { message.as_mut_vec() }, buffer);
                if message.is_empty() {
                    // if a 0-length reply, then we're done after the reply and shall break out
                    return;
                }
            }
        }
    }
}

pub(crate) fn init_pools() {
    unsafe {
        REQ_POOL.set(SyncPool::new());
        RESP_POOL.set(SyncPool::new());
        POOL_CHAN.set(channel::bounded(0))
    }

    thread::spawn(|| {
        let cap = TOTAL_ELEM_COUNT / 5;
        let mut count = 0;

        loop {
            thread::sleep(Duration::from_secs(1));
            count += 1;

            if let Ok(chan) = unsafe { POOL_CHAN.as_ref() } {
                match chan.1.try_recv() {
                    Ok(_) | Err(TryRecvError::Disconnected) => return,
                    _ => {}
                }
            } else {
                // shouldn't happen, but we shall quit now
                return;
            }

            if count % 30 == 0 {
                if let Ok(pool) = unsafe { REQ_POOL.as_mut() } {
                    if pool.len() < cap {
                        pool.refill(cap);
                    }
                }

                if let Ok(pool) = unsafe { RESP_POOL.as_mut() } {
                    if pool.len() < cap {
                        pool.refill(cap);
                    }
                }

                count = 0;
            }
        }
    });
}

pub(crate) fn drop_statics() {
    unsafe {
        if let Ok(chan) = POOL_CHAN.as_ref() {
            // zero-sized channel will block until the message is read, which shall happen evey second.
            chan.0.send(()).unwrap_or_default();
        }

        // take the pools out
        let _ = POOL_CHAN.take();
        let mut req_pool = REQ_POOL.take();
        let mut resp_pool = RESP_POOL.take();

        // drop them in place
        ptr::drop_in_place(&mut req_pool as *mut Option<SyncPool<Request>>);
        ptr::drop_in_place(&mut resp_pool as *mut Option<SyncPool<Response>>);
    }
}

fn broadcast_new_communications(sender: Sender<String>, mut stream_clone: Stream) {
    thread::spawn(move || {
        let mut buffer = [0u8; 512];

        loop {
            if let Err(e) = stream_clone.take_error() {
                debug::print(
                    &format!("Keep-alive stream can't continue: {}", e),
                    InfoLevel::Warning,
                );
                break;
            }

            if let Err(e) = stream_clone.read(&mut buffer) {
                debug::print(
                    &format!("Unable to continue reading from a keep-alive stream: {}", e),
                    InfoLevel::Warning,
                );
                break;
            }

            if let Ok(result) = str::from_utf8(&buffer) {
                if result.is_empty() {
                    continue;
                }

                if let Err(err) = sender.send(result.to_owned()) {
                    // this could be caused by shutting down the stream from the main thread, so more of
                    // the informative level of the message.
                    debug::print(
                        &format!("Unable to broadcast the communications: {}", err),
                        InfoLevel::Error,
                    );
                    break;
                }
            }
        }
    });
}

fn stream_trunk(content: &[u8], buffer: &mut BufWriter<&mut Stream>) {
    // the content length should have been set in the header, see function resp_header
    write_to_buff(buffer, content.len().to_string().as_bytes());
    write_line_break(buffer);
    write_to_buff(buffer, content);
    write_line_break(buffer);
    flush_buffer(buffer);
}

fn stream_default_body(status: u16, buffer: &mut BufWriter<&mut Stream>) {
    match status {
        //explicit error status
        0 | 404 => write_default_page(400, buffer),
        500 => write_default_page(500, buffer),
        _ => { /* Nothing */ }
    };
}

fn write_default_page(status: u16, buffer: &mut BufWriter<&mut Stream>) {
    match status {
        500 => {
            /* return default 500 page */
            write_to_buff(buffer, FIVE_HUNDRED.as_bytes());
        }
        _ => {
            /* return default/override 404 page */
            write_to_buff(buffer, FOUR_OH_FOUR.as_bytes());
        }
    }
}

fn get_file_path(path: &str) -> Option<PathBuf> {
    if path.is_empty() {
        debug::print(
            "Undefined file path to retrieve data from...",
            InfoLevel::Warning,
        );
        return None;
    }

    let file_path = Path::new(path);
    if !file_path.is_file() {
        debug::print("Can't locate requested file", InfoLevel::Warning);
        return None;
    }

    Some(file_path.to_path_buf())
}

fn open_file(file_path: &PathBuf, buf: &mut Vec<u8>) -> u16 {
    // try open the file
    if let Ok(file) = File::open(file_path) {
        let mut buf_reader = BufReader::new(file);
        match buf_reader.read_to_end(buf) {
            Err(e) => {
                debug::print(&format!("Unable to read file: {}", e), InfoLevel::Warning);
                500
            }
            Ok(_) => {
                //things are truly ok now
                200
            }
        }
    } else {
        debug::print("Unable to open requested file for path", InfoLevel::Warning);
        404
    }
}

fn open_file_async(file_path: PathBuf, tx: Sender<(Vec<u8>, u16)>) {
    assert!(file_path.is_file());

    shared_pool::run(
        move || {
            // try open the file
            if let Ok(file) = File::open(file_path) {
                let mut buf_reader = BufReader::new(file);
                let mut buf = Vec::with_capacity(1024);

                match buf_reader.read_to_end(&mut buf) {
                    Ok(len) => {
                        if tx.send((buf, 200)).is_err() {
                            debug::print(
                                "Unable to write the file to the stream",
                                InfoLevel::Warning,
                            );
                        }
                    }
                    Err(e) => {
                        debug::print(&format!("Unable to read file: {}", e), InfoLevel::Warning);
                    }
                }
            } else {
                debug::print("Unable to open requested file for path", InfoLevel::Warning);
            }
        },
        TaskType::Response,
    );
}

fn get_status(status: u16) -> Vec<u8> {
    let status = match status {
        100 => "100 Continue",
        101 => "101 Switching Protocols",
        200 => "200 OK",
        201 => "201 Created",
        202 => "202 Accepted",
        203 => "203 Non-Authoritative Information",
        204 => "204 No Content",
        205 => "205 Reset Content",
        206 => "206 Partial Content",
        300 => "300 Multiple Choices",
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

    let mut result = Vec::with_capacity(11 + status.len());
    result.extend_from_slice(b"HTTP/1.1 ");
    result.extend_from_slice(status.as_bytes());
    result.append_line_break();

    result
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
        "docx" => {
            String::from("application/vnd.openxmlformats-officedocument.wordprocessingml.document")
        }
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
        "pptx" => String::from(
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        ),
        "rar" => String::from("application/x-rar-compressed"),
        "swf" => String::from("application/x-shockwave-flash"),
        "vsd" => String::from("application/vnd.visio"),
        "wasm" => String::from("application/wasm"),
        "weba" => String::from("audio/webm"),
        "xhtml" => String::from("application/xhtml+xml"),
        "xul" => String::from("application/vnd.mozilla.xul+xml"),
        "7z" => String::from("application/x-7z-compressed"),
        "svg" => String::from("image/svg+xml"),
        "csh" | "sh" | "tar" | "wav" => ["application/x-", ext].join(""),
        "csv" | "html" | "htm" => ["text/", ext].join(""),
        "jpeg" | "jpg" | "gif" | "png" | "bmp" | "webp" | "tiff" | "tif" => {
            ["image/", ext].join("")
        }
        "otf" | "ttf" | "woff" | "woff2" => ["font/", ext].join(""),
        "midi" | "mp3" | "aac" | "mid" | "oga" => ["audio/", ext].join(""),
        "webm" | "mp4" | "ogg" | "mpeg" | "ogv" => ["video/", ext].join(""),
        "xml" | "pdf" | "json" | "ogx" | "rtf" | "zip" => ["application/", ext].join(""),
        _ if !ext.is_empty() => ["application/", ext].join(""),
        _ => String::from("text/plain"),
    }
}

fn write_header_status(status: u16, has_contents: bool) -> Vec<u8> {
    match status {
        404 | 500 => get_status(status),
        0 => {
            /* No status has been explicitly set, be smart here */
            if has_contents {
                get_status(200)
            } else {
                get_status(404)
            }
        }
        _ => {
            /* A status has been set explicitly, respect that here. */
            get_status(status)
        }
    }
}

fn write_headers(source: &HashMap<String, String>, header: &mut Vec<u8>, keep_alive: bool) {
    header.reserve_exact(24);
    header.extend_from_slice(b"Server: Rusty-Express/");
    header.extend_from_slice(VERSION.as_bytes());
    header.append_line_break();

    if !source.contains_key("date") {
        let dt = Utc::now().format("%a, %e %b %Y %T GMT").to_string();

        header.reserve_exact(8);
        header.extend_from_slice(b"Date: ");
        header.extend_from_slice(dt.as_bytes());
        header.append_line_break();
    }

    let transfer = String::from("transfer-encoding");
    for (field, value) in source.iter() {
        header.reserve_exact(field.len() + value.len() + 4);
        header.extend_from_slice(field.as_bytes());
        header.extend_from_slice(b": ");
        header.extend_from_slice(value.as_bytes());

        if keep_alive && field.eq(&transfer) && !value.contains("chunked") {
            header.reserve_exact(9);
            header.extend_from_slice(b", chunked\r\n");
        } else {
            header.append_line_break();
        }
    }
}

fn write_header_cookie(cookie: HashMap<String, Cookie>, tx: Sender<Vec<u8>>) {
    let mut output = Vec::new();

    for (_, cookie) in cookie.iter() {
        if cookie.is_valid() {
            let c = cookie.to_string();

            output.reserve_exact(14 + c.len());
            output.extend_from_slice(b"Set-Cookie: ");
            output.extend_from_slice(c.as_bytes());
            output.append_line_break();
        }
    }

    tx.send(output).unwrap_or_else(|e| {
        debug::print(
            &format!("Unable to write response cookies: {}", e),
            InfoLevel::Warning,
        );
    });
}
