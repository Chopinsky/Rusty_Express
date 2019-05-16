//! Rusty Express is a simple server written in Rust and provide Express-alike APIs.
//! This project aims to provide a http server solution which is easy to use, easy to
//! scale, and is excellent on performance.
//!
//! # Examples
//! ```
//! extern crate rusty_express as express;
//! use express::prelude::{HttpServer, RequestPath, Request, Response};
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
//! ```

#![allow(unused_variables)]
#[macro_use]

extern crate lazy_static;
extern crate chrono;
extern crate hashbrown;
extern crate native_tls;
extern crate num_cpus;
extern crate parking_lot;
extern crate rand;
extern crate regex;

pub(crate) mod core;
pub(crate) mod support;

pub mod prelude {
    pub use crate::core::config::{
        EngineContext, PageGenerator, ServerConfig, ViewEngine, ViewEngineDefinition,
    };

    pub use crate::core::context as ServerContext;
    pub use crate::core::context::ContextProvider;
    pub use crate::core::cookie::*;
    pub use crate::core::http::{Request, RequestWriter, Response, ResponseStates, ResponseWriter};
    pub use crate::core::router::{RequestPath, Route, Router, REST};
    pub use crate::core::states::ControlMessage;
    pub use crate::{HttpServer, ServerDef};

    #[cfg(feature = "session")]
    pub use crate::support::session::*;

    #[cfg(feature = "logger")]
    pub use crate::support::logger::InfoLevel;
}

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use self::core::config::{ServerConfig, ViewEngine, ViewEngineDefinition};
use self::core::conn::*;
use self::core::router::*;
use self::core::states::*;
use self::support::debug::{self, InfoLevel};
use self::support::session::*;
use self::support::{shared_pool, ThreadPool};
use crossbeam_channel as channel;
use hashbrown::HashMap;
use native_tls::TlsAcceptor;

//TODO: Impl middlewear

pub struct HttpServer {
    config: ServerConfig,
    state: ServerStates,
}

impl HttpServer {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn new_with_config(config: ServerConfig) -> Self {
        HttpServer {
            config,
            state: ServerStates::new(),
        }
    }

    /// `listen` will take 1 parameter for the port that the server will be monitoring at, aka
    /// `127.0.0.1:port`. This function will block until the server is shut down.
    ///
    /// # Examples
    ///
    /// ```rust
    /// extern crate rusty_express as express;
    /// use express::prelude::{HttpServer};
    ///
    /// let mut server = HttpServer::new();
    /// server.def_router(router);
    /// server.listen(8080);
    /// ```
    pub fn listen(&mut self, port: u16) {
        // delegate the actual work to the more robust routine.
        self.listen_and_serve(port, None);
    }

    pub fn listen_and_serve(
        &mut self,
        port: u16,
        callback: Option<fn(channel::Sender<ControlMessage>)>,
    ) {
        // initialize the debug service, which setup the debug level based on the environment variable
        debug::initialize();

        // create the listener
        let server_address = SocketAddr::from(([127, 0, 0, 1], port));
        let listener = TcpListener::bind(server_address).unwrap_or_else(|err| {
            panic!("Unable to start the http server: {}...", err);
        });

        // obtain the control message courier service and start the callback
        let control_handler =
            callback.map(|cb| {
                let sender = self.state.get_courier_sender();

                thread::spawn(move || {
                    // sleep 100 ms to avoid racing with the main server before it's ready to take control messages.
                    thread::sleep(Duration::from_millis(96));
                    cb(sender);
                })
            });

        // launch the service, now this will block until the server is shutdown
        println!("Listening for connections on port {}", port);
        self.launch_with(&listener);

        // start to shut down the TcpListener
        println!("Shutting down...");

        // now terminate the callback function as well.
        if let Some(handler) = control_handler {
            handler.join().unwrap_or_else(|err| {
                debug::print(
                    "Failed to shut down the callback handler, the service is teared down correctly",
                    InfoLevel::Warning
                );
            });
        }
    }

    #[inline]
    #[must_use]
    pub fn get_courier(&self) -> channel::Sender<ControlMessage> {
        self.state.get_courier_sender()
    }

    #[inline]
    pub fn drop_session_auto_clean(&mut self) {
        self.state.drop_session_auto_clean();
    }

    pub fn config_hot_reload(&self) {
        if !self.state.is_running() {
            eprintln!("The function is meant to be used for hot-loading a new server configuration when it's running...");
            return;
        }

        if self
            .state
            .courier_deliver(ControlMessage::HotReloadConfig)
            .is_err()
        {
            debug::print("Failed to hot reload the configuration", InfoLevel::Error);
        }
    }

    fn launch_with(&mut self, listener: &TcpListener) {
        // if using the session module and allow auto clean up, launch the service now.
        self.session_cleanup_config();
        self.state.toggle_running_state(true);

        //TODO: impl TLS setup ...
        let acceptor: Option<Arc<TlsAcceptor>> = None;
        let pool_size = self.config.get_pool_size();

        let mut workers_pool = setup_worker_pools(pool_size);
        workers_pool.toggle_auto_expansion(true);

        for stream in listener.incoming() {
            if let Some(message) = self.state.courier_fetch() {
                match message {
                    ControlMessage::Terminate => {
                        if let Ok(s) = stream {
                            send_err_resp(Stream::Tcp(s), 503);
                        }

                        break;
                    }
                    ControlMessage::HotReloadConfig => {
                        self.session_cleanup_config();
                    }
                    ControlMessage::HotLoadRouter(r) => {
                        if let Err(err) = Route::use_router(r) {
                            debug::print(
                                &format!(
                                    "An error has taken place when trying to update the router: {}",
                                    err
                                )[..],
                                InfoLevel::Warning,
                            );
                        }
                    }
                    ControlMessage::HotLoadConfig(c) => {
                        if c.get_pool_size() != self.config.get_pool_size() {
                            debug::print(
                                "Change size of the thread pool is not supported while the server is running",
                                InfoLevel::Warning
                            );
                        }

                        self.config = c;
                        self.session_cleanup_config();
                    }
                    ControlMessage::Custom(content) => {
                        println!("The message: {} is not yet supported.", content)
                    }
                }
            }

            match stream {
                Ok(s) => {
                    self.handle_stream(s, &mut workers_pool, &acceptor);
                },
                Err(e) => debug::print(
                    &format!("Failed to receive the upcoming stream: {}", e)[..],
                    InfoLevel::Warning,
                ),
            }
        }

        // must close the shared pool, since it's a static and won't drop with the end of the server,
        // which could cause response executions still on-the-fly to crash.
        shared_pool::close();

        self.state.toggle_running_state(false);
    }

    fn handle_stream(&self, stream: TcpStream, workers_pool: &mut ThreadPool, acceptor: &Option<Arc<TlsAcceptor>>) {
        let read_timeout = u64::from(self.config.get_read_timeout());
        let write_timeout = u64::from(self.config.get_write_timeout());

        let acceptor_clone = if let Some(acceptor) = acceptor {
            Some(acceptor.clone())
        } else {
            None
        };

        workers_pool.execute(move || {
            stream.set_timeout(read_timeout, write_timeout);
            if let Some(a) = acceptor_clone {
                // handshake and encrypt
                match a.accept(stream) {
                    Ok(s) => {
                        handle_connection(Stream::Tls(Box::new(s)));
                    },
                    Err(e) => debug::print(
                        &format!("Failed to receive the upcoming stream: {:?}", e)[..],
                        InfoLevel::Error,
                    ),
                };
            } else{
                handle_connection(Stream::Tcp(stream));
            }
        });
    }

    fn session_cleanup_config(&mut self) {
        if self.config.get_session_auto_clean() && !ExchangeConfig::auto_clean_is_running() {
            if let Some(duration) = self.config.get_session_auto_clean_period() {
                self.state
                    .set_session_handler(ExchangeConfig::auto_clean_start(duration));
            }
        }
    }
}

impl Default for HttpServer {
    fn default() -> Self {
        // reset the router with the new server instance
        Route::init();

        HttpServer {
            config: ServerConfig::default(),
            state: ServerStates::new(),
        }
    }
}

fn setup_worker_pools(size: usize) -> ThreadPool {
    shared_pool::initialize_with(vec![size]);
    ThreadPool::new(size)
}

trait StreamTimeoutConfig {
    fn set_timeout(&self, read_timeout: u64, write_timeout: u64);
}

impl StreamTimeoutConfig for TcpStream {
    fn set_timeout(&self, read_timeout: u64, write_timeout: u64) {
        if read_timeout > 0 {
            self.set_read_timeout(Some(Duration::from_millis(read_timeout)))
                .unwrap_or_else(|err| {
                    debug::print(
                        &format!("Unable to set read timeout: {}", err)[..],
                        InfoLevel::Warning,
                    );
                });
        }

        if write_timeout > 0 {
            self.set_write_timeout(Some(Duration::from_millis(write_timeout)))
                .unwrap_or_else(|err| {
                    debug::print(
                        &format!("Unable to set write timeout: {}", err)[..],
                        InfoLevel::Warning,
                    );
                });
        }
    }
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
        if let Err(err) = Route::use_router(router) {
            eprintln!(
                "An error has taken place when trying to update the router: {}",
                err
            );
        }
    }

    fn set_pool_size(&mut self, size: usize) {
        if self.state.is_running() {
            eprintln!(
                "Change size of the thread pool is not supported while the server is running"
            );
            return;
        }

        self.config.set_pool_size(size);
    }

    fn set_read_timeout(&mut self, timeout: u16) {
        self.config.set_read_timeout(timeout);

        if self.state.is_running() {
            self.config_hot_reload();
        }
    }

    fn set_write_timeout(&mut self, timeout: u16) {
        self.config.set_write_timeout(timeout);

        if self.state.is_running() {
            self.config_hot_reload();
        }
    }

    fn def_default_response_header(&mut self, header: HashMap<String, String>) {
        ServerConfig::use_default_header(header);
    }

    fn set_default_response_header(&mut self, field: String, value: String) {
        ServerConfig::set_default_header(field, value, true);
    }

    fn enable_session_auto_clean(&mut self, auto_clean_period: Duration) {
        self.config.set_session_auto_clean_period(auto_clean_period);

        if self.state.is_running() {
            self.config_hot_reload();
        }
    }

    fn disable_session_auto_clean(&mut self) {
        self.config.clear_session_auto_clean();

        if self.state.is_running() {
            self.config_hot_reload();
        }
    }
}

impl Router for HttpServer {
    fn get(&mut self, uri: RequestPath, callback: Callback) -> &mut Self {
        Route::add_route(REST::GET, uri, RouteHandler::new(Some(callback), None));
        self
    }

    fn patch(&mut self, uri: RequestPath, callback: Callback) -> &mut Self {
        Route::add_route(REST::PATCH, uri, RouteHandler::new(Some(callback), None));
        self
    }

    fn post(&mut self, uri: RequestPath, callback: Callback) -> &mut Self {
        Route::add_route(REST::POST, uri, RouteHandler::new(Some(callback), None));
        self
    }

    fn put(&mut self, uri: RequestPath, callback: Callback) -> &mut Self {
        Route::add_route(REST::PUT, uri, RouteHandler::new(Some(callback), None));
        self
    }

    fn delete(&mut self, uri: RequestPath, callback: Callback) -> &mut Self {
        Route::add_route(REST::DELETE, uri, RouteHandler::new(Some(callback), None));
        self
    }

    fn options(&mut self, uri: RequestPath, callback: Callback) -> &mut Self {
        Route::add_route(REST::OPTIONS, uri, RouteHandler::new(Some(callback), None));
        self
    }

    fn other(&mut self, method: &str, uri: RequestPath, callback: Callback) -> &mut Self {
        Route::add_route(REST::OTHER(
            method.to_uppercase()), uri, RouteHandler::new(Some(callback), None)
        );

        self
    }

    fn all(&mut self, uri: RequestPath, callback: Callback) -> &mut Self {
        self.other("*", uri, callback);
        self
    }

    /// # Example
    ///
    /// ```
    /// extern crate rusty_express;
    /// use rusty_express::prelude::*;
    /// use std::path::PathBuf;
    /// fn main() {
    ///    // define http server now
    ///    let mut server = HttpServer::new();
    ///    server.set_pool_size(8);
    ///    server.use_static(PathBuf::from(r".\static"));
    /// }
    /// ```
    fn use_static(&mut self, path: PathBuf) -> &mut Self {
        Route::add_static(REST::GET, None, path);
        self
    }

    fn use_custom_static(&mut self, uri: RequestPath, path: PathBuf) -> &mut Self {
        Route::add_static(REST::GET, Some(uri), path);
        self
    }
}

impl ViewEngineDefinition for HttpServer {
    #[inline]
    fn view_engine(extension: &str, engine: ViewEngine) {
        ServerConfig::view_engine(extension, engine);
    }
}
