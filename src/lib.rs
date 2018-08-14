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

#![allow(deprecated)]
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
    pub use support::logger::LogLevel;
}

use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use core::config::{ConnMetadata, ServerConfig, ViewEngine, ViewEngineDefinition};
use core::connection::*;
use core::router::*;
use core::states::*;
use support::debug;
use support::session::*;
use support::{shared_pool, ThreadPool};

//TODO: Impl middlewear

static mut READ_TIMEOUT: Option<Duration> = None;
static mut WRITE_TIMEOUT: Option<Duration> = None;

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
        unsafe {
            READ_TIMEOUT = Some(Duration::from_millis(self.config.read_timeout as u64));
            WRITE_TIMEOUT = Some(Duration::from_millis(self.config.write_timeout as u64));
        }

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

    #[deprecated(
        since = "0.3.3", note = "use server courier to send the termination message instead."
    )]
    pub fn try_to_terminate(&mut self) {
        debug::print("Requested to shutdown...", 0);
        self.state.ack_to_terminate();
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

    fn launch_with(&mut self, listener: &TcpListener) {
        // if using the session module and allow auto clean up, launch the service now.
        session_auto_config(&self.config, &mut self.state);

        let workers_pool = ThreadPool::new(self.config.pool_size);
        shared_pool::initialize_with(vec![self.config.pool_size]);

        let mut shared_router = Arc::new(self.router.to_owned());
        let mut shared_metadata = Arc::new(self.config.get_meta_data());

        for stream in listener.incoming() {
            if let Some(message) = self.state.courier_try_recv() {
                match message {
                    ControlMessage::Terminate => {
                        if let Ok(s) = stream {
                            let conn_meta = Arc::clone(&shared_metadata);
                            send_err_resp(s, 503, conn_meta);
                        }

                        break;
                    }
                    ControlMessage::HotLoadRouter(r) => {
                        self.router = r;
                        shared_router = Arc::new(self.router.to_owned());
                    }
                    ControlMessage::HotLoadConfig(c) => {
                        self.config = c;
                        session_auto_config(&self.config, &mut self.state);
                        shared_metadata = Arc::new(self.config.get_meta_data());
                    }
                    ControlMessage::Custom(content) => {
                        println!("The message: {} is not yet supported.", content)
                    }
                }
            }

            match stream {
                Ok(s) => handle_stream(s, &shared_router, &shared_metadata, &workers_pool),
                Err(e) => debug::print(
                    &format!("Failed to receive the upcoming stream: {}", e)[..],
                    1,
                ),
            }
        }

        // must close the shared pool, since it's a static and won't drop with the end of the server,
        // which could cause response executions still on-the-fly to crash.
        shared_pool::close();
    }
}

fn handle_stream(
    stream: TcpStream,
    router: &Arc<Route>,
    meta: &Arc<ConnMetadata>,
    workers_pool: &ThreadPool,
) {
    // clone Arc-pointers
    let router_ptr = Arc::clone(&router);
    let meta_ptr = Arc::clone(&meta);

    workers_pool.execute(move || {
        unsafe {
            stream.set_timeout(READ_TIMEOUT, WRITE_TIMEOUT);
        }
        handle_connection(stream, router_ptr, meta_ptr);
    });
}

fn session_auto_config(config: &ServerConfig, state: &mut ServerStates) {
    if config.use_session_autoclean && !ExchangeConfig::auto_clean_is_running() {
        if let Some(duration) = config.get_session_auto_clean_period() {
            state.set_session_handler(ExchangeConfig::auto_clean_start(duration));
        }
    }
}

trait StreamTimeoutConfig {
    fn set_timeout(&self, read_timeout: Option<Duration>, write_timeout: Option<Duration>);
}

impl StreamTimeoutConfig for TcpStream {
    fn set_timeout(&self, read_timeout: Option<Duration>, write_timeout: Option<Duration>) {
        self.set_read_timeout(read_timeout).unwrap_or_else(|err| {
            debug::print(&format!("Unable to set read timeout: {}", err)[..], 1);
        });

        self.set_write_timeout(write_timeout).unwrap_or_else(|err| {
            debug::print(&format!("Unable to set write timeout: {}", err)[..], 1);
        });
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
