//! Rusty Express is a simple server written in Rust and provide Express-alike APIs.
//! This project aims to provide a http server solution which is easy to use, easy to
//! scale, and is excellent on performance.
//!
//! # Examples
//! ```
//! use rusty_express::prelude::*;
//!
//! let mut server = HttpServer::new();
//! server.get(RequestPath::Explicit("/"), simple_response);
//! server.listen(8080);
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
extern crate regex;

#[cfg(feature = "session")]
extern crate rand;

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
    pub use crate::core::server::{HttpServer, ServerDef};
    pub use crate::core::states::{AsyncController, ControlMessage};

    #[cfg(feature = "session")]
    pub use crate::support::session::*;

    #[cfg(feature = "logger")]
    pub use crate::support::logger::InfoLevel;
}

use crossbeam_channel as channel;
