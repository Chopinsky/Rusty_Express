#[macro_use]
extern crate lazy_static;
extern crate regex;
extern crate chrono;
extern crate rand;

pub mod config;
pub mod connection;
pub mod http;
pub mod router;
pub mod server_states;
pub mod session;
pub mod thread_utils;

pub mod prelude {
    pub use {HttpServer, ServerDef};
    pub use config::*;
    pub use http::{Request, Response, ResponseWriter};
    pub use router::{REST, Route, Router, RequestPath};
}

use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener};
use std::time::{Duration};

use config::*;
use connection::*;
use router::*;
use server_states::*;
use session::*;
use thread_utils::ThreadPool;

//TODO: 1. handle errors with grace...
//TODO: 2. Impl middlewear
//TODO: 3. Use lazy_static to create session control

pub struct HttpServer {
    router: Route,
    config: ServerConfig,
    states: ServerStates,
}

pub trait ServerDef {
    fn def_router(&mut self, router: Route);
    fn set_pool_size(&mut self, size: usize);
    fn set_read_timeout(&mut self, timeout: u8);
    fn set_write_timeout(&mut self, timeout: u8);
    fn def_default_response_header(&mut self, header: HashMap<String, String>);
    fn set_default_response_header(&mut self, field: String, value: String);
}

impl HttpServer {
    pub fn new() -> Self {
        HttpServer {
            router: Route::new(),
            config: ServerConfig::new(),
            states: ServerStates::new(),
        }
    }

    pub fn listen(&self, port: u16) {
        let server_address = SocketAddr::from(([127, 0, 0, 1], port));
        if let Ok(listener) = TcpListener::bind(server_address) {
            println!("Listening for connections on port {}", port);

            start_with(&listener, &self.router, &self.config, &self.states);
            drop(listener);
        } else {
            panic!("Unable to start the http server...");
        }

        println!("Shutting down...");
    }

    pub fn try_to_terminate(&mut self) {
        println!("Requested to shutdown...");
        self.states.ack_to_terminate();
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

impl ServerDef for HttpServer {
    fn def_router(&mut self, router: Route) {
        self.router = router;
    }

    fn set_pool_size(&mut self, size: usize) {
        self.config.pool_size = size;
    }

    fn set_read_timeout(&mut self, timeout: u8) {
        self.config.read_timeout = timeout;
    }

    fn set_write_timeout(&mut self, timeout: u8) {
        self.config.write_timeout = timeout;
    }

    fn def_default_response_header(&mut self, header: HashMap<String, String>) {
        self.config.use_default_header(&header);
    }

    fn set_default_response_header(&mut self, field: String, value: String) {
        self.config.default_header(field, value, true);
    }
}

fn start_with(listener: &TcpListener, router: &Route, config: &ServerConfig, server_states: &ServerStates) {

    let pool = ThreadPool::new(config.pool_size);
    let read_timeout = Some(Duration::new(config.read_timeout as u64, 0));
    let write_timeout = Some(Duration::new(config.write_timeout as u64, 0));

    for stream in listener.incoming() {
        if let Ok(s) = stream {
//            let session = Session::new();
//            println!("Session id: {}", session.get_id());

            if let Err(e) = s.set_read_timeout(read_timeout) {
                println!("Unable to set read timeout: {}", e);
                continue;
            }

            if let Err(e) = s.set_write_timeout(write_timeout) {
                println!("Unable to set write timeout: {}", e);
                continue;
            }

            // clone the router so it can out live the closure.
            let router = Route::from(&router);
            let default_header = config.get_default_header();
            let default_pages = config.get_default_pages();

            pool.execute(move || {
                handle_connection(s, &router, &default_header, &default_pages);
            });
        }

        if server_states.is_terminating() {
            break;
        }
    }

    //close the listener with grace
    drop(pool);
}
