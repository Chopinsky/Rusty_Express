use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::channel;
use crate::core::{
    config::{ServerConfig, ViewEngine, ViewEngineDefinition},
    conn::*,
    router::*,
    states::*,
    stream::*,
};
use crate::hashbrown::HashMap;
use crate::native_tls::TlsAcceptor;
use crate::support::{
    debug::{self, InfoLevel},
    session::*,
    shared_pool, ThreadPool, TimeoutPolicy,
};

//TODO: Impl middlewear

/// The server instance that represents and controls the underlying http-service.
pub struct HttpServer {
    config: ServerConfig,
    state: ServerStates,
}

impl HttpServer {
    /// Create a new server instance using default configurations and server settings.
    pub fn new() -> Self {
        Default::default()
    }

    /// Create a new server instance with supplied configuration and settings.
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
    /// use express::prelude::{HttpServer, ServerDef, Router};
    ///
    /// let mut server = HttpServer::new();
    /// server.def_router(router);
    /// server.listen(8080);
    /// ```
    pub fn listen(&mut self, port: u16) {
        // delegate the actual work to the more robust routine.
        self.listen_and_serve(port, None);
    }

    /// `listen_and_serve` will take 2 parameters: 1) the port that the server will be monitoring at,
    ///  or `127.0.0.1:port`; 2) the callback closure that will take an async-controller as input,
    /// and run in parallel to the current server instance for async operations.
    ///
    /// This function will block until the server is shut down.
    ///
    /// # Examples
    ///
    /// ```rust
    /// extern crate rusty_express as express;
    /// use express::prelude::{HttpServer, ServerDef, Router, ControlMessage};
    /// use std::thread;
    /// use std::time::Duration;
    ///
    /// let mut server = HttpServer::new();
    /// server.def_router(router);
    /// server.listen_and_serve(8080, |controller| {
    ///     // sleep for 1 minute
    ///     thread::sleep(Duration::from_secs(60));
    ///
    ///     // after waking up from the 1 minute sleep, shut down the server.
    ///     controller.send(ControlMessage::Terminate);
    /// });
    /// ```
    pub fn listen_and_serve(&mut self, port: u16, callback: Option<fn(AsyncController)>) {
        // initialize the debug service, which setup the debug level based on the environment variable
        debug::initialize();

        // update the server state for the socket-host address
        self.state.set_port(port);

        // create the listener
        let server_address = SocketAddr::from(([127, 0, 0, 1], port));
        let listener = TcpListener::bind(server_address).unwrap_or_else(|err| {
            panic!("Unable to start the http server: {}...", err);
        });

        // obtain the control message courier service and start the callback
        let (control_handler, controller_tx) = if let Some(cb) = callback {
            let sender = self.state.get_courier_sender();
            let (tx, rx) = channel::bounded(1);

            let handler = thread::spawn(move || {
                // wait for server to launch before it's ready to take control messages.
                let _ = rx.recv();
                cb(sender);
            });

            (Some(handler), Some(tx))
        } else {
            (None, None)
        };

        // launch the service, now this will block until the server is shutdown
        println!("Listening for connections on port {}", port);

        // actually mounting the server
        self.launch_with(&listener, controller_tx);

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

    /// Obtain an `AsyncController`, which can be run in a parallel thread and control or update
    /// server configurations while it's running. For more details, see the `states` module.
    #[inline]
    #[must_use]
    pub fn get_courier(&self) -> AsyncController {
        self.state.get_courier_sender()
    }

    #[inline]
    /// Stop and clear the session auto-cleaning schedules. This API can be useful when no more new
    /// sessions shall be built and stored in the server, to save server resources.
    pub fn drop_session_auto_clean(&mut self) {
        self.state.drop_session_auto_clean();
    }

    /// Obtain a reference to the server config, such that we can make updates **before** launching
    /// the server but without creating the config struct and pass it in on building the server.
    pub fn config(&mut self) -> &mut ServerConfig {
        &mut self.config
    }

    /// Ask the server to reload the configuration settings. Usually used in a separate thread with
    /// a cloned server instance, where the server state is corrupted and need a reload to restore the
    /// initial server settings.
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

    fn launch_with(&mut self, listener: &TcpListener, mut cb_sig: Option<channel::Sender<()>>) {
        // if using the session module and allow auto clean up, launch the service now.
        if cfg!(feature = "session") {
            self.session_cleanup_config();
        }

        self.state.toggle_running_state(true);

        let acceptor: Option<Arc<TlsAcceptor>> = self.config.build_tls_acceptor();
        let (mut read_timeout, mut write_timeout, mut req_limit) = self.config.load_server_params();

        let mut workers_pool = self.setup_worker_pools();
        workers_pool.toggle_auto_expansion(true, None);
        workers_pool.set_timeout_policy(TimeoutPolicy::Run);

        // notify the server launcher that we're ready to serve incoming streams
        if let Some(sender) = cb_sig.take() {
            sender.send(()).unwrap_or_else(|err| {
                debug::print(
                    &format!(
                        "Failed to notify the server launching callback function: {}",
                        err
                    )[..],
                    InfoLevel::Warning,
                );
            });
        }

        for stream in listener.incoming() {
            if let Some(message) = self.state.fetch_update() {
                match message {
                    ControlMessage::Terminate => {
                        if let Ok(s) = stream {
                            send_err_resp(Stream::Tcp(s), 503);
                        }

                        break;
                    }
                    ControlMessage::HotReloadConfig => {
                        if cfg!(feature = "session") {
                            self.session_cleanup_config();
                        }
                    }
                    ControlMessage::HotLoadRouter(r) => {
                        Route::use_router_async(r);
                    }
                    ControlMessage::HotLoadConfig(c) => {
                        // check pool size param
                        if c.get_pool_size() != self.config.get_pool_size() {
                            debug::print(
                                "Change size of the thread pool is not supported while the server is running",
                                InfoLevel::Warning
                            );
                        }

                        // load the bulk params and decompose
                        let params = c.load_server_params();
                        read_timeout = params.0;
                        write_timeout = params.1;
                        req_limit = params.2;

                        // update the config and reset the session clean effort
                        self.config = c;

                        if cfg!(feature = "session") {
                            self.session_cleanup_config();
                        }
                    }
                    ControlMessage::Custom(content) => {
                        println!("The message: {} is not yet supported.", content)
                    }
                }
            }

            match stream {
                Ok(s) => {
                    // set the timeout for this connection
                    if read_timeout > 0 || write_timeout > 0 {
                        s.set_timeout(read_timeout, write_timeout);
                    }

                    // process the connection
                    self.handle_stream(
                        s,
                        &mut workers_pool,
                        acceptor.clone(),
                        read_timeout,
                        write_timeout,
                        req_limit,
                    );
                }
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

    fn handle_stream(
        &self,
        stream: TcpStream,
        workers_pool: &mut ThreadPool,
        acceptor: Option<Arc<TlsAcceptor>>,
        read_timeout: u64,
        write_timeout: u64,
        req_limit: usize,
    ) {
        workers_pool.execute(move || {
            if let Some(a) = acceptor {
                // handshake and encrypt
                match a.accept(stream) {
                    Ok(s) => {
                        Stream::Tls(Box::new(s)).process(true, req_limit);
                    }
                    Err(e) => debug::print(
                        &format!("Failed to receive the upcoming stream: {:?}", e)[..],
                        InfoLevel::Error,
                    ),
                };
            } else {
                Stream::Tcp(stream).process(false, req_limit);
            }
        });
    }

    #[cfg(feature = "session")]
    fn session_cleanup_config(&mut self) {
        if !self.config.get_session_auto_clean() || ExchangeConfig::auto_clean_is_running() {
            return;
        }

        if let Some(duration) = self.config.get_session_auto_clean_period() {
            self.state
                .set_session_handler(ExchangeConfig::auto_clean_start(duration));
        }
    }

    fn setup_worker_pools(&self) -> ThreadPool {
        let size = self.config.get_pool_size();
        shared_pool::initialize_with(vec![size]);
        ThreadPool::new(size)
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

trait TimeoutConfig {
    fn set_timeout(&self, read_timeout: u64, write_timeout: u64);
}

impl TimeoutConfig for TcpStream {
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
    /// Replace the default server router with the pre-built one. This is a wrapper over
    /// `Route::use_router`, which will achieve same goal.
    fn def_router(&mut self, router: Route) {
        Route::use_router(router);
    }

    /// Set the pool size. Note that this API is only functional before launching the server. After
    /// the server has started and been running, `set_pool_size` will be no-op.
    fn set_pool_size(&mut self, size: usize) {
        if self.state.is_running() {
            eprintln!(
                "Change size of the thread pool is not supported while the server is running"
            );

            return;
        }

        self.config.set_pool_size(size);
    }

    /// Set the read timeout for the handler. If no more incoming data stream are detected on the
    /// socket, the read stream will be closed.
    fn set_read_timeout(&mut self, timeout: u16) {
        self.config.set_read_timeout(timeout);

        if self.state.is_running() {
            self.config_hot_reload();
        }
    }

    /// Set the write timeout for the handler. If no more outgoing data stream are detected on the
    /// socket, the write stream will be closed.
    fn set_write_timeout(&mut self, timeout: u16) {
        self.config.set_write_timeout(timeout);

        if self.state.is_running() {
            self.config_hot_reload();
        }
    }

    /// Define headers and their contents that shall go along with every http response. This will
    /// remove any existing default headers and corresponding contents.
    fn def_default_response_header(&mut self, header: HashMap<String, String>) {
        ServerConfig::use_default_header(header);
    }

    /// Set or update default headers and their contents that shall go along with every http response.
    /// If a default header with same name exists, the new contents will replace the existing one.
    fn set_default_response_header(&mut self, field: String, value: String) {
        ServerConfig::set_default_header(field, value, true);
    }

    /// If using the `session` feature, this API will automatically purge stale session stores, such
    /// that we can reclaim resources that's no longer in use.
    #[cfg(feature = "session")]
    fn enable_session_auto_clean(&mut self, auto_clean_period: Duration) {
        self.config.set_session_auto_clean_period(auto_clean_period);

        if self.state.is_running() {
            self.config_hot_reload();
        }
    }

    /// If using the `session` feature, this API will turn off the periodic session store clean up
    #[cfg(feature = "session")]
    fn disable_session_auto_clean(&mut self) {
        self.config.clear_session_auto_clean();

        if self.state.is_running() {
            self.config_hot_reload();
        }
    }
}

impl Router for HttpServer {
    fn get(&mut self, uri: RequestPath, callback: Callback) -> &mut dyn Router {
        Route::add_route(REST::GET, uri, RouteHandler::new(Some(callback), None));
        self
    }

    fn patch(&mut self, uri: RequestPath, callback: Callback) -> &mut dyn Router {
        Route::add_route(REST::PATCH, uri, RouteHandler::new(Some(callback), None));
        self
    }

    fn post(&mut self, uri: RequestPath, callback: Callback) -> &mut dyn Router {
        Route::add_route(REST::POST, uri, RouteHandler::new(Some(callback), None));
        self
    }

    fn put(&mut self, uri: RequestPath, callback: Callback) -> &mut dyn Router {
        Route::add_route(REST::PUT, uri, RouteHandler::new(Some(callback), None));
        self
    }

    fn delete(&mut self, uri: RequestPath, callback: Callback) -> &mut dyn Router {
        Route::add_route(REST::DELETE, uri, RouteHandler::new(Some(callback), None));
        self
    }

    fn options(&mut self, uri: RequestPath, callback: Callback) -> &mut dyn Router {
        Route::add_route(REST::OPTIONS, uri, RouteHandler::new(Some(callback), None));
        self
    }

    fn other(&mut self, method: &str, uri: RequestPath, callback: Callback) -> &mut dyn Router {
        Route::add_route(
            REST::OTHER(method.to_uppercase()),
            uri,
            RouteHandler::new(Some(callback), None),
        );

        self
    }

    fn all(&mut self, uri: RequestPath, callback: Callback) -> &mut dyn Router {
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
    fn use_static(&mut self, path: PathBuf) -> &mut dyn Router {
        Route::add_static(REST::GET, None, path);
        self
    }

    fn use_custom_static(&mut self, uri: RequestPath, path: PathBuf) -> &mut dyn Router {
        Route::add_static(REST::GET, Some(uri), path);
        self
    }

    fn case_sensitive(&mut self, allow_case: bool, method: Option<REST>) {
        if method.is_none() {
            Route::all_case_sensitive(allow_case);
            return;
        }

        if let Some(m) = method {
            Route::case_sensitive(&m, allow_case);
        }
    }
}

impl ViewEngineDefinition for HttpServer {
    #[inline]
    fn view_engine(extension: &str, engine: ViewEngine) {
        ServerConfig::view_engine(extension, engine);
    }
}
