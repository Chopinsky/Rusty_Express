#[macro_use]
extern crate lazy_static;
extern crate regex;
extern crate chrono;
extern crate rand;

mod core;
mod support;

pub mod prelude {
    pub use {HttpServer, ServerDef};
    pub use core::config::*;
    pub use core::cookie::*;
    pub use core::http::{Request, Response, ResponseStates, ResponseWriter};
    pub use core::router::{REST, Route, Router, RequestPath};
    pub use support::session::*;
}

use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use core::config::ServerConfig;
use core::connection::*;
use core::router::*;
use core::states::*;
use support::session::*;
use support::ThreadPool;

//TODO: 1. handle errors with grace...
//TODO: 2. Impl middlewear

pub struct HttpServer<T: Send + Sync + Clone + 'static> {
    router: Route,
    config: ServerConfig,
    states: ServerStates,
    managed: ManagedStates<T>,
}

impl<T: Send + Sync + Clone + 'static> HttpServer<T> {
    pub fn new() -> Self {
        HttpServer {
            router: Route::new(),
            config: ServerConfig::new(),
            states: ServerStates::new(),
            managed: ManagedStates::new(),
        }
    }

    pub fn new_with_config(config: ServerConfig) -> Self {
        HttpServer {
            router: Route::new(),
            config,
            states: ServerStates::new(),
            managed: ManagedStates::new(),
        }
    }

    pub fn manage(&mut self, key: &str, state: T) {
        self.managed.add_state(key.to_owned(), state);
    }

    pub fn listen(&mut self, port: u16) {
        let server_address = SocketAddr::from(([127, 0, 0, 1], port));
        if let Ok(listener) = TcpListener::bind(server_address) {
            println!("Listening for connections on port {}", port);

            if self.config.use_session_autoclean && !Session::auto_clean_is_running() {
                if let Some(duration) = self.config.get_session_auto_clean_period() {
                    let handler = Session::auto_clean_start(duration);
                    self.states.set_session_handler(&handler);
                }
            }

            start_with(&listener, &self.router, &self.config,
                       &self.states, &self.managed);
        } else {
            panic!("Unable to start the http server...");
        }

        println!("Shutting down...");
    }

    pub fn try_to_terminate(&mut self) {
        println!("Requested to shutdown...");
        self.states.ack_to_terminate();
    }

    pub fn drop_session_auto_clean(&mut self) {
        self.states.drop_session_auto_clean();
    }
}

impl<T: Send + Sync + Clone + 'static> Router for HttpServer<T> {
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

pub trait ServerDef {
    fn def_router(&mut self, router: Route);
    fn set_pool_size(&mut self, size: usize);
    fn set_read_timeout(&mut self, timeout: u8);
    fn set_write_timeout(&mut self, timeout: u8);
    fn def_default_response_header(&mut self, header: HashMap<String, String>);
    fn set_default_response_header(&mut self, field: String, value: String);
    fn enable_session_auto_clean(&mut self, auto_clean_period: Duration);
    fn disable_session_auto_clean(&mut self);
}

fn start_with<T: Send + Sync + Clone + 'static>(
    listener: &TcpListener,
    router: &Route,
    config: &ServerConfig,
    server_states: &ServerStates,
    managed_states: &ManagedStates<T>) {

    let pool = ThreadPool::new(config.pool_size);
    let read_timeout = Some(Duration::new(config.read_timeout as u64, 0));
    let write_timeout = Some(Duration::new(config.write_timeout as u64, 0));

    let meta_data = Arc::new(config.get_meta_data());
    let managed = Arc::new(Mutex::new(managed_states.to_owned()));
    let router = Arc::new(router.to_owned());

    for stream in listener.incoming() {

//        if let Some(mut session) = Session::new() {
//            session.expires_at(SystemTime::now().add(Duration::new(5, 0)));
//            session.save();
//            println!("New session: {}", session.get_id());
//        }

        if let Ok(s) = stream {
            // clone Arc-pointers
            let stream_router = Arc::clone(&router);
            let conn_handler = Arc::clone(&meta_data);
            let states = Arc::clone(&managed);

            pool.execute(move || {
                if let Err(e) = s.set_read_timeout(read_timeout) {
                    println!("Unable to set read timeout: {}", e);
                }

                if let Err(e) = s.set_write_timeout(write_timeout) {
                    println!("Unable to set write timeout: {}", e);
                }

                handle_connection(s, stream_router, conn_handler, states);
            });
        }

        if server_states.is_terminating() {
            return;
        }
    }
}

impl<T: Send + Sync + Clone + 'static> ServerDef for HttpServer<T> {
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
        self.config.set_default_header(field, value, true);
    }

    fn enable_session_auto_clean(&mut self, auto_clean_period: Duration) {
        self.config.set_session_auto_clean(auto_clean_period);
    }

    fn disable_session_auto_clean(&mut self) {
        self.config.reset_session_auto_clean();
    }
}
