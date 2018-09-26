//! Rusty Express is a simple server written in Rust and provide Express-alike APIs.
//! This project aims to provide a http server solution which is easy to use, easy to
//! scale, and is excellent on performance.
//!
//! # Examples
//! ```
//! extern crate rusty_express as express;
//! use express::prelude::*;
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
extern crate crossbeam_channel as channel;
extern crate num_cpus;
extern crate rand;
extern crate regex;

pub(crate) mod core;
pub(crate) mod support;

pub mod prelude {
    pub use core::config::{
        EngineContext, PageGenerator, ServerConfig, ViewEngine, ViewEngineDefinition,
    };

    pub use core::context as ServerContext;
    pub use core::context::ContextProvider;
    pub use core::cookie::*;
    pub use core::http::{Request, RequestWriter, Response, ResponseStates, ResponseWriter};
    pub use core::router::{RequestPath, Route, Router, REST};
    pub use core::states::ControlMessage;
    pub use {HttpServer, ServerDef};

    #[cfg(feature = "session")]
    pub use support::session::*;

    #[cfg(feature = "logger")]
    pub use support::logger::InfoLevel;
}

use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use core::config::{ServerConfig, ViewEngine, ViewEngineDefinition};
use core::connection::*;
use core::router::*;
use core::states::*;
use support::debug;
use support::session::*;
use support::{shared_pool, ThreadPool};

//TODO: Impl middlewear

pub struct HttpServer {
    router: Route,
    config: ServerConfig,
    state: ServerStates,
}

impl HttpServer {
    pub fn new() -> Self {
        HttpServer {
            router: Route::new(),
            config: ServerConfig::new(),
            state: ServerStates::new(),
        }
    }

    pub fn new_with_config(config: ServerConfig) -> Self {
        HttpServer {
            router: Route::new(),
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
    /// use express::prelude::*;
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
        let control_handler = if let Some(cb) = callback {
            let sender = self.state.get_courier_sender();
            Some(thread::spawn(move || {
                // sleep 100 ms to avoid racing with the main server before it's ready to take control messages.
                thread::sleep(Duration::from_millis(100));
                cb(sender);
            }))
        } else {
            None
        };

        // launch the service, now this will block until the server is shutdown
        println!("Listening for connections on port {}", port);
        self.launch_with(&listener);

        // start to shut down the TcpListener
        println!("Shutting down...");

        // now terminate the callback function as well.
        if let Some(handler) = control_handler {
            handler.join().unwrap_or_else(|err| {
                debug::print("Failed to shut down the callback handler, the service is teared down correctly", 1);
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

        let sender = self.state.get_courier_sender();
        sender.send(ControlMessage::HotLoadConfig(self.config.clone()));
    }

    fn launch_with(&mut self, listener: &TcpListener) {
        // if using the session module and allow auto clean up, launch the service now.
        self.session_cleanup_config();

        let workers_pool = setup_worker_pools(&self.config.get_pool_size());
        self.state.toggle_running_state(true);

        for stream in listener.incoming() {
            if let Some(message) = self.state.courier_try_recv() {
                match message {
                    ControlMessage::Terminate => {
                        if let Ok(s) = stream {
                            send_err_resp(s, 503);
                        }

                        break;
                    },
                    ControlMessage::HotLoadRouter(r) => {
                        if let Err(err) = Route::use_router(r) {
                            eprintln!("An error has taken place when trying to update the router: {}", err);
                        }
                    },
                    ControlMessage::HotLoadConfig(c) => {
                        if c.get_pool_size() != self.config.get_pool_size() {
                            eprintln!("Change size of the thread pool is not supported while the server is running");
                        }

                        self.config = c;
                        self.session_cleanup_config();
                    },
                    ControlMessage::Custom(content) => {
                        println!("The message: {} is not yet supported.", content)
                    },
                }
            }

            match stream {
                Ok(s) => self.handle_stream(s, &workers_pool),
                Err(e) => debug::print(
                    &format!("Failed to receive the upcoming stream: {}", e)[..],
                    1,
                ),
            }
        }

        // must close the shared pool, since it's a static and won't drop with the end of the server,
        // which could cause response executions still on-the-fly to crash.
        shared_pool::close();

        self.state.toggle_running_state(false);
    }

    fn handle_stream(
        &self,
        stream: TcpStream,
        workers_pool: &ThreadPool,
    ) {
        // clone Arc-pointers
        let read_timeout = self.config.get_read_timeout() as u64;
        let write_timeout = self.config.get_write_timeout() as u64;

        workers_pool.execute(move || {
            stream.set_timeout(read_timeout, write_timeout);
            handle_connection(stream);
        });
    }

    fn session_cleanup_config(&mut self) {
        if self.config.get_session_auto_clean() && !ExchangeConfig::auto_clean_is_running() {
            if let Some(duration) = self.config.get_session_auto_clean_period() {
                self.state.set_session_handler(ExchangeConfig::auto_clean_start(duration));
            }
        }
    }
}

fn setup_worker_pools(size: &usize) -> ThreadPool {
    shared_pool::initialize_with(vec![*size]);
    ThreadPool::new(*size)
}

trait StreamTimeoutConfig {
    fn set_timeout(&self, read_timeout: u64, write_timeout: u64);
}

impl StreamTimeoutConfig for TcpStream {
    fn set_timeout(&self, read_timeout: u64, write_timeout: u64) {
        if read_timeout > 0 {
            self.set_read_timeout(Some(Duration::from_millis(read_timeout))).unwrap_or_else(|err| {
                debug::print(&format!("Unable to set read timeout: {}", err)[..], 1);
            });
        }

        if write_timeout > 0 {
            self.set_write_timeout(Some(Duration::from_millis(write_timeout))).unwrap_or_else(|err| {
                debug::print(&format!("Unable to set write timeout: {}", err)[..], 1);
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
            eprintln!("An error has taken place when trying to update the router: {}", err);
        }
    }

    fn set_pool_size(&mut self, size: usize) {
        if self.state.is_running() {
            eprintln!("Change size of the thread pool is not supported while the server is running");
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
    fn get(&mut self, uri: RequestPath, callback: Callback) -> &mut Route {
        self.router.get(uri, callback);
        &mut self.router
    }

    fn patch(&mut self, uri: RequestPath, callback: Callback) -> &mut Route {
        self.router.patch(uri, callback);
        &mut self.router
    }

    fn post(&mut self, uri: RequestPath, callback: Callback) -> &mut Route {
        self.router.post(uri, callback);
        &mut self.router
    }

    fn put(&mut self, uri: RequestPath, callback: Callback) -> &mut Route {
        self.router.put(uri, callback);
        &mut self.router
    }

    fn delete(&mut self, uri: RequestPath, callback: Callback) -> &mut Route {
        self.router.delete(uri, callback);
        &mut self.router
    }

    fn options(&mut self, uri: RequestPath, callback: Callback) -> &mut Route {
        self.router.options(uri, callback);
        &mut self.router
    }

    fn other(&mut self, method: &str, uri: RequestPath, callback: Callback) -> &mut Route {
        self.router.other(method, uri, callback);
        &mut self.router
    }

    fn all(&mut self, uri: RequestPath, callback: Callback) -> &mut Route {
        self.router.all(uri, callback);
        &mut self.router
    }
}

impl ViewEngineDefinition for HttpServer {
    #[inline]
    fn view_engine(extension: &str, engine: ViewEngine) {
        ServerConfig::view_engine(extension, engine);
    }
}
