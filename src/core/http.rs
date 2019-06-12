#![allow(dead_code)]

use std::fs::File;
use std::io::prelude::*;
use std::io::{BufReader, BufWriter};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::str;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::channel;
use crate::core::config::{ConnMetadata, EngineContext, ServerConfig, ViewEngineParser};
use crate::core::cookie::*;
use crate::core::router::REST;
use crate::core::stream::Stream;
use crate::support::{common::*, debug, debug::InfoLevel, shared_pool, TaskType};
use chrono::prelude::*;
use hashbrown::{hash_map::Iter, HashMap};

static FOUR_OH_FOUR: &'static str = include_str!("../default/404.html");
static FOUR_OH_ONE: &'static str = include_str!("../default/401.html");
static FIVE_HUNDRED: &'static str = include_str!("../default/500.html");
static VERSION: &'static str = env!("CARGO_PKG_VERSION");

static RESP_TIMEOUT: Duration = Duration::from_millis(64);
static LONG_CONN_TIMEOUT: Duration = Duration::from_secs(8);

type BodyChan = (Option<Arc<channel::Sender<(String, u16)>>>, Option<channel::Receiver<(String, u16)>>);
type NotifyChan = Option<(channel::Sender<String>, channel::Receiver<String>)>;

//TODO: internal registered cache?

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

pub struct Request {
    pub method: REST,
    pub uri: String,
    header: HashMap<String, String>,
    cookie: HashMap<String, String>,
    body: Vec<String>,
    scheme: HashMap<String, Vec<String>>,
    fragment: String,
    params: HashMap<String, String>,
    host: String,
    client_info: Option<SocketAddr>,
}

impl Request {
    #[inline]
    pub fn new() -> Self {
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

    pub fn scheme(&self, field: &str) -> Option<Vec<String>> {
        if field.is_empty() {
            return None;
        }

        if self.scheme.is_empty() {
            return None;
        }

        match self.scheme.get(&field[..]) {
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

    pub fn json(&self) -> String {
        let mut source = HashMap::new();

        source.insert(String::from("method"), self.method.to_string());
        source.insert(String::from("uri"), self.uri.to_owned());

        if !self.params.is_empty() {
            source.insert(String::from("uri_params"), json_stringify(&self.params));
        }

        if !self.scheme.is_empty() {
            source.insert(
                String::from("uri_schemes"),
                json_flat_stringify(&self.scheme),
            );
        }

        if !self.fragment.is_empty() {
            source.insert(String::from("uri_fragment"), self.fragment.to_owned());
        }

        if !self.body.is_empty() {
            source.insert(String::from("body"), (&self.body).flat());
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

    pub(crate) fn set_bodies(&mut self, body: Vec<String>) {
        self.body = body;
    }
}

impl Default for Request {
    fn default() -> Self {
        Request {
            method: REST::GET,
            uri: String::new(),
            header: HashMap::new(),
            cookie: HashMap::new(),
            body: Vec::new(),
            scheme: HashMap::new(),
            fragment: String::new(),
            params: HashMap::new(),
            host: String::new(),
            client_info: None,
        }
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
    fn set_fragment(&mut self, fragment: String);
    fn set_host(&mut self, host: String);
    fn set_client(&mut self, addr: SocketAddr);
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
        self.body.push(content.to_owned());
    }
}

#[derive(Default)]
pub struct Response {
    status: u16,
    keep_alive: KeepAliveStatus,
    content_type: String,
    content_length: Option<String>,
    cookie: Arc<HashMap<String, Cookie>>,
    header: HashMap<String, String>,
    header_only: bool,
    redirect: String,
    body: String,
    body_chan: BodyChan,
    notifier: NotifyChan,
    subscriber: NotifyChan,
}

impl Response {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn new_with_default_header(default_header: HashMap<String, String>) -> Self {
        let mut response = Response::new();
        response.header = default_header;
        response
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

    fn resp_header(&self) -> String {
        // Get cookie parser to its own thread
        let receiver: Option<channel::Receiver<String>> = if self.cookie.is_empty() {
            None
        } else {
            let (tx, rx) = channel::unbounded();
            let cookie = Arc::clone(&self.cookie);

            shared_pool::run(
                move || {
                    write_header_cookie(cookie, tx);
                },
                TaskType::Response,
            );

            Some(rx)
        };

        let mut header = write_header_status(self.status, self.has_contents());

        // other header field-value pairs
        write_headers(
            &self.header,
            &mut header,
            self.to_keep_alive(),
        );

        if !self.content_type.is_empty() {
            header.reserve_exact(16 + self.content_type.len());
            header.push_str("Content-Type: ");
            header.push_str(&self.content_type);
            header.append_line_break();
        }

        if let Some(length) = self.content_length.as_ref() {
            // explicit content length is set, use it here
            header.reserve_exact(18 + length.len());
            header.push_str("Content-Length: ");
            header.push_str(&length);
            header.append_line_break();
        } else {
            // Only generate content length header attribute if not using async and no content-length set explicitly
            if self.is_header_only() || self.body.is_empty() {
                header.reserve_exact(19);
                header.push_str("Content-Length: 0\r\n");
            } else {
                let size = self.body.len().to_string();
                header.reserve_exact(18 + size.len());
                header.push_str("Content-Length: ");
                header.push_str(&size);
                header.append_line_break();
            }
        }

        if !self.header.contains_key("connection") {
            let (connection, count) = if self.to_keep_alive() {
                ("keep-alive", 10)
            } else {
                ("close", 5)
            };

            header.reserve_exact(14 + count);
            header.push_str("Connection: ");
            header.push_str(connection);
            header.append_line_break();
        }

        if let Some(rx) = receiver {
            if let Ok(content) = rx.recv_timeout(RESP_TIMEOUT) {
                if !content.is_empty() {
                    header.push_str(&content);
                }
            }
        }

        // if header only, we're done, write the new line as the EOF
        header
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

pub trait ResponseStates {
    fn to_keep_alive(&self) -> bool;
    fn get_redirect_path(&self) -> String;
    fn get_header(&self, key: &str) -> Option<&String>;
    fn get_cookie(&self, key: &str) -> Option<&Cookie>;
    fn get_content_type(&self) -> String;
    fn status_is_set(&self) -> bool;
    fn has_contents(&self) -> bool;
    fn is_header_only(&self) -> bool;
    fn get_channels(
        &mut self,
    ) -> Result<(channel::Sender<String>, channel::Receiver<String>), &'static str>;
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
    fn get_channels(
        &mut self,
    ) -> Result<(channel::Sender<String>, channel::Receiver<String>), &'static str> {
        if self.notifier.is_none() {
            self.notifier = Some(channel::unbounded());
        }

        if self.subscriber.is_none() {
            self.subscriber = Some(channel::unbounded());
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
    fn send(&mut self, content: &str);
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
    fn status(&mut self, status: u16) {
        self.status = match status {
            100...101 => status,
            200...206 => status,
            300...308 if status != 307 && status != 308 => status,
            400...417 if status != 402 => status,
            426 | 428 | 429 | 431 | 451 => status,
            500...505 | 511 => status,
            _ => 0,
        };
    }

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
                    self.content_length = Some(value.to_owned());
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
                    },
                    "tls" => self.keep_alive = KeepAliveStatus::TlsConn,
                    "forbidden" => self.keep_alive = KeepAliveStatus::Forbidden,
                    _ => { /* Otherwise, don't update the keep_alive field */ },
                }
            }
            _ => {
                self.header.add(field, value.to_owned(), allow_replace);
            }
        };
    }

    /// set_header is a sugar to the `header` API, and it's created to simplify the majority of the
    /// use cases for setting the response header.
    ///
    /// By default, the field-value pair will be set to the response header, and override any existing
    /// pairs if they've been set prior to the API call.
    ///
    /// # Examples
    /// resp.set_header("Content-Type", "application/javascript");
    /// assert!(resp.get_header("Content-Type").unwrap(), &String::from("application/javascript"));
    #[inline]
    fn set_header(&mut self, field: &str, value: &str) {
        self.header(field, value, true);
    }

    fn send(&mut self, content: &str) {
        if self.is_header_only() {
            return;
        }

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
            self.body = String::new();
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
            let (tx, rx) = channel::bounded(64);
            self.body_chan = (Some(Arc::new(tx)), Some(rx));
        }

        // set header's mime extension field
        self.set_ext_mime_header(&path);

        // actually load the file to the response body
        if let Some(chan) = self.body_chan.0.as_ref() {
            open_file_async(path, Arc::clone(chan));
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

            let mut content = String::new();
            open_file(&path, &mut content);

            // Now render the conent with the engine
            let status = ServerConfig::template_parser(&ext[..], &mut content, context);
            if status == 0 || status == 200 {
                self.body = content;
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

        if let Some(cookie_set) = Arc::get_mut(&mut self.cookie) {
            let key = cookie.get_cookie_key();
            cookie_set.insert(key, cookie);
        }
    }

    fn set_cookies(&mut self, cookies: &[Cookie]) {
        if let Some(cookie_set) = Arc::get_mut(&mut self.cookie) {
            for cookie in cookies.iter() {
                if !cookie.is_valid() {
                    continue;
                }

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

    #[inline]
    fn can_keep_alive(&mut self, can_keep_alive: bool) {
        self.keep_alive = if !can_keep_alive {
            KeepAliveStatus::Forbidden
        } else {
            KeepAliveStatus::NotSet
        }
    }

    fn keep_alive(&mut self, to_keep: bool) {
        if self.keep_alive == KeepAliveStatus::TlsConn || self.keep_alive == KeepAliveStatus::Forbidden {
            return;
        }

        self.keep_alive = if to_keep { KeepAliveStatus::KeepAlive } else { KeepAliveStatus::NotSet };
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
    fn write_header(&self, buffer: &mut BufWriter<&mut Stream>);
    fn write_body(&self, buffer: &mut BufWriter<&mut Stream>);
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
            // manual drop the transmission channel so we won't hang forever
            drop(self.body_chan.0.take());

            // try to receive the async bodies
            if let Some(chan) = self.body_chan.1.take() {
                for received in chan {
                    if received.1 == 200 {
                        // read the content
                        self.body.push_str(&received.0);
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
                    self.body = page_generator();
                } else {
                    self.body = FOUR_OH_FOUR.to_owned();
                }
            }
            401 => {
                if let Some(page_generator) = ConnMetadata::get_status_pages(401) {
                    self.body = page_generator();
                } else {
                    self.body = FOUR_OH_ONE.to_owned();
                }
            }
            _ => {
                if let Some(page_generator) = ConnMetadata::get_status_pages(500) {
                    self.body = page_generator();
                } else {
                    self.body = FIVE_HUNDRED.to_owned();
                }
            }
        }
    }

    fn write_header(&self, buffer: &mut BufWriter<&mut Stream>) {
        write_to_buff(buffer, self.resp_header().as_bytes());
    }

    fn write_body(&self, buffer: &mut BufWriter<&mut Stream>) {
        if self.has_contents() {
            // the content length should have been set in the header, see function resp_header
            write_to_buff(buffer, self.body.as_bytes());
        } else {
            // this shouldn't happen, as we should have captured this in the check_and_update call
            stream_default_body(self.status, buffer);
        }
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
            while let Ok(message) = notifier.1.recv_timeout(LONG_CONN_TIMEOUT) {
                stream_trunk(&message, buffer);
                if message.is_empty() {
                    // if a 0-length reply, then we're done after the reply and shall break out
                    return;
                }
            }
        }
    }
}

fn broadcast_new_communications(sender: channel::Sender<String>, mut stream_clone: Stream) {
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

fn stream_trunk(content: &str, buffer: &mut BufWriter<&mut Stream>) {
    // the content length should have been set in the header, see function resp_header
    write_to_buff(buffer, content.len().to_string().as_bytes());
    write_line_break(buffer);
    write_to_buff(buffer, content.as_bytes());
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

fn open_file(file_path: &PathBuf, buf: &mut String) -> u16 {
    // try open the file
    if let Ok(file) = File::open(file_path) {
        let mut buf_reader = BufReader::new(file);
        return match buf_reader.read_to_string(buf) {
            Err(e) => {
                debug::print(&format!("Unable to read file: {}", e), InfoLevel::Warning);
                500
            }
            Ok(_) => {
                //things are truly ok now
                200
            }
        };
    } else {
        debug::print("Unable to open requested file for path", InfoLevel::Warning);
        404
    }
}

fn open_file_async(file_path: PathBuf, tx: Arc<channel::Sender<(String, u16)>>) {
    shared_pool::run(move || {
        assert!(file_path.is_file());

        // try open the file
        if let Ok(file) = File::open(file_path) {
            let mut buf_reader = BufReader::new(file);
            let mut buf = [0u8; 512];

            loop {
                match buf_reader.read(&mut buf) {
                    Ok(len) => {
                        if let Ok(content) = str::from_utf8(&buf[..len]) {
                            if let Err(e) = tx.send((content.to_owned(), 200)) {
                                debug::print(
                                    &format!("Unable to write the file to the stream: {}", e),
                                    InfoLevel::Warning,
                                );
                                break;
                            }
                        } else {
                            if let Err(e) = tx.send((String::new(), 500)) {
                                debug::print(
                                    &format!("Unable to write the file to the stream: {}", e),
                                    InfoLevel::Warning,
                                );
                            }

                            break;
                        }

                        if len < 512 {
                            break;
                        }
                    },
                    Err(e) => {
                        debug::print(&format!("Unable to read file: {}", e), InfoLevel::Warning);
                        break;
                    },
                }
            }
        } else {
            debug::print("Unable to open requested file for path", InfoLevel::Warning);
        }
    }, TaskType::Response);
}

fn get_status(status: u16) -> String {
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

    ["HTTP/1.1 ", status, "\r\n"].join("")
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

fn write_header_status(status: u16, has_contents: bool) -> String {
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

fn write_headers(source: &HashMap<String, String>, header: &mut String, keep_alive: bool) {
    header.reserve_exact(24);
    header.push_str("Server: Rusty-Express/");
    header.push_str(VERSION);
    header.append_line_break();

    if !source.contains_key("date") {
        let dt = Utc::now().format("%a, %e %b %Y %T GMT").to_string();

        header.reserve_exact(8);
        header.push_str("Date: ");
        header.push_str(&dt);
        header.append_line_break();
    }

    let transfer = String::from("transfer-encoding");
    for (field, value) in source.iter() {
        header.reserve_exact(field.len() + value.len() + 4);
        header.push_str(field);
        header.push_str(": ");
        header.push_str(value);

        if keep_alive && field.eq(&transfer) && !value.contains("chunked") {
            header.reserve_exact(9);
            header.push_str(", chunked\r\n");
        } else {
            header.push_str("\r\n");
        }
    }
}

fn write_header_cookie(cookie: Arc<HashMap<String, Cookie>>, tx: channel::Sender<String>) {
    let mut output = String::new();
    for (_, cookie) in cookie.iter() {
        if cookie.is_valid() {
            let c = cookie.to_string();

            output.reserve_exact(14 + c.len());
            output.push_str("Set-Cookie: ");
            output.push_str(&c);
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
