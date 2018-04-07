#![allow(deprecated)]

//! Rusty Express is a simple server written in Rust and provide Express-alike APIs.
//! This project aims to provide a http server solution which is easy to use, easy to
//! scale, and is excellent on performance.
//!
//! # Examples
//! extern crate rusty_express;
//! use rusty_express::prelude::*;
//!
//! fn main() {
//!    let mut server = HttpServer::new();
//!     server.get(RequestPath::Explicit("/"), simple_response);
//!    server.listen(8080);
//! }
//!
//! pub fn simple_response(req: &Box<Request>, resp: &mut Box<Response>) {
//!    resp.send(&format!("Hello world from rusty server from path: {}", req.uri));
//!    resp.status(200);
//! }

#[macro_use]
extern crate lazy_static;
extern crate regex;
extern crate chrono;
extern crate rand;
extern crate num_cpus;

mod core;
mod support;

pub mod prelude {
    pub use {HttpServer, ServerDef};
    pub use core::config::{EngineContext, PageGenerator, ServerConfig, ViewEngineDefinition, ViewEngine};
    pub use core::context::ContextProvider;
    pub use core::context as ServerContext;
    pub use core::cookie::*;
    pub use core::http::{Request, RequestWriter, Response, ResponseStates, ResponseWriter};
    pub use core::router::{REST, Route, Router, RequestPath};
    pub use core::states::{StatesProvider, StatesInteraction, RequireStateUpdates};

    #[cfg(feature = "session")]
    pub use support::session::*;
}

use std::collections::HashMap;
use std::net::{SocketAddr, Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use core::config::{ServerConfig, ViewEngineDefinition, ViewEngine};
use core::connection::*;
use core::router::*;
use core::states::*;
use support::debug;
use support::session::*;
use support::{ThreadPool, shared_pool};

//TODO: 1. logger? or middlewear?
//TODO: 2. Impl middlewear
//TODO: 3. remove States Management related features on 0.3.4

pub struct HttpServer {
    router: Route,
    config: ServerConfig,
    states: ServerStates,
}

impl HttpServer {
    pub fn new() -> Self {
        HttpServer {
            router: Route::new(),
            config: ServerConfig::new(),
            states: ServerStates::new(),
        }
    }

    pub fn new_with_config(config: ServerConfig) -> Self {
        HttpServer {
            router: Route::new(),
            config,
            states: ServerStates::new(),
        }
    }

    pub fn listen(&mut self, port: u16) {
        debug::initialize();

        let server_address = SocketAddr::from(([127, 0, 0, 1], port));
        let listener = TcpListener::bind(server_address).unwrap_or_else(|err| {
            panic!("Unable to start the http server: {}...", err);
        });

        println!("Listening for connections on port {}", port);

        if self.config.use_session_autoclean && !Session::auto_clean_is_running() {
            if let Some(duration) = self.config.get_session_auto_clean_period() {
                if let Some(handler) = Session::auto_clean_start(duration) {
                    self.states.set_session_handler(&handler);
                }
            }
        }

        start_with(&listener, &self.router, &self.config, &self.states);
        println!("Shutting down...");
    }

    #[deprecated(since = "0.3.0", note = "This feature will be removed in 0.3.3")]
    pub fn listen_and_manage<T: Send + Sync + Clone + StatesProvider + 'static>(&mut self, port: u16, state: Arc<RwLock<T>>) {
        self.listen(port);
    }

    pub fn try_to_terminate(&mut self) {
        debug::print("Requested to shutdown...", 0);
        self.states.ack_to_terminate();
    }

    pub fn drop_session_auto_clean(&mut self) {
        self.states.drop_session_auto_clean();
    }
}

fn start_with(
        listener: &TcpListener,
        router: &Route,
        config: &ServerConfig,
        server_states: &ServerStates) {

    let workers_pool = ThreadPool::new(config.pool_size);
    shared_pool::initialize_with(vec![config.pool_size]);

    let read_timeout = Some(Duration::from_millis(config.read_timeout as u64));
    let write_timeout = Some(Duration::from_millis(config.write_timeout as u64));

    let router = Arc::new(router.to_owned());
    let meta_arc = Arc::new(config.get_meta_data());

    for stream in listener.incoming() {
        if let Ok(s) = stream {
            if server_states.is_terminating() {
                // Told to close the connection, shut down the socket now.
                &s.shutdown(Shutdown::Both).unwrap_or_else(|e| {
                    debug::print(&format!("Unable to shut down the stream: {}", e)[..], 1);
                });

                return;
            }

            // clone Arc-pointers
            let router_ptr = Arc::clone(&router);
            let meta_ptr = Arc::clone(&meta_arc);

            workers_pool.execute(move || {
                set_timeout(&s, read_timeout, write_timeout);
                handle_connection(s, router_ptr, meta_ptr);
            });
        }
    }

    // must close the shared pool, since it's a static and won't drop with the end of the server,
    // which could cause response executions still on-the-fly to crash.
    shared_pool::close();
}

fn set_timeout(stream: &TcpStream, read: Option<Duration>, write: Option<Duration>) {
    stream.set_read_timeout(read).unwrap_or_else(|err| {
        debug::print(&format!("Unable to set read timeout: {}", err)[..], 1);
    });

    stream.set_write_timeout(write).unwrap_or_else(|err| {
        debug::print(&format!("Unable to set write timeout: {}", err)[..], 1);
    });
}

pub trait ServerDef {
    fn def_router(&mut self, router: Route);
    fn set_pool_size(&mut self, size: usize);
    fn set_read_timeout(&mut self, timeout: u16);
    fn set_write_timeout(&mut self, timeout: u16);
    fn def_default_response_header(&mut self, header: HashMap<String, String>);
    fn set_default_response_header(&mut self, field: String, value: String);
    fn enable_session_auto_clean(&mut self, auto_clean_period: Duration);
    fn disable_session_auto_clean(&mut self);
}

impl ServerDef for HttpServer {
    fn def_router(&mut self, router: Route) {
        self.router = router;
    }

    fn set_pool_size(&mut self, size: usize) {
        self.config.pool_size = size;
    }

    fn set_read_timeout(&mut self, timeout: u16) {
        self.config.read_timeout = timeout;
    }

    fn set_write_timeout(&mut self, timeout: u16) {
        self.config.write_timeout = timeout;
    }

    fn def_default_response_header(&mut self, header: HashMap<String, String>) {
        self.config.use_default_header(header);
    }

    fn set_default_response_header(&mut self, field: String, value: String) {
        self.config.set_default_header(field, value, true);
    }

    fn enable_session_auto_clean(&mut self, auto_clean_period: Duration) {
        self.config.set_session_auto_clean(auto_clean_period);
    }

    fn disable_session_auto_clean(&mut self) {
        self.config.reset_session_auto_clean();
    }
}

impl Router for HttpServer {
    fn get(&mut self, uri: RequestPath, callback: Callback) {
        self.router.get(uri, callback);
    }

    fn post(&mut self, uri: RequestPath, callback: Callback) {
        self.router.post(uri, callback);
    }

    fn put(&mut self, uri: RequestPath, callback: Callback) {
        self.router.put(uri, callback);
    }

    fn delete(&mut self, uri: RequestPath, callback: Callback) {
        self.router.delete(uri, callback);
    }

    fn options(&mut self, uri: RequestPath, callback: Callback) {
        self.router.options(uri, callback);
    }

    fn other(&mut self, method: &str, uri: RequestPath, callback: Callback) {
        self.router.other(method, uri, callback);
    }
}

impl ViewEngineDefinition for HttpServer {
    #[inline]
    fn view_engine(extension: &str, engine: ViewEngine) {
        ServerConfig::view_engine(extension, engine);
    }
}
