#![allow(dead_code)]

use std::sync::RwLock;
use super::http::{Request, Response};

lazy_static! {
    static ref CONTEXT: RwLock<Box<ServerContextProvider>> = RwLock::new(Box::new(EmptyContext {}));
}

pub type ServerContextProvider = ContextProvider + Sync + Send + 'static;

pub fn set_context(context: Box<ServerContextProvider>) {
    if let Ok(mut c) = CONTEXT.write() {
        *c = context;
    }
}

pub fn update_context(req: &Box<Request>, resp: &mut Box<Response>) -> Result<(), &'static str> {
    if let Ok(mut c) = CONTEXT.write() {
        return c.update(req, resp);
    }

    Err("Unable to lock and update the context")
}

pub fn process_with_context(req: &Box<Request>, resp: &mut Box<Response>) -> Result<(), &'static str> {
    if let Ok(c) = CONTEXT.read() {
        return c.process(req, resp);
    }

    Err("Unable to lock and process the response with the context")
}

pub trait ContextProvider {
    fn update(&mut self, req: &Box<Request>, resp: &mut Box<Response>) -> Result<(), &'static str>;
    fn process(&self, req: &Box<Request>, resp: &mut Box<Response>) -> Result<(), &'static str>;
}

struct EmptyContext {}

impl ContextProvider for EmptyContext {
    #[inline]
    fn update(&mut self, _req: &Box<Request>, _resp: &mut Box<Response>) -> Result<(), &'static str> { Ok(()) }

    #[inline]
    fn process(&self, _req: &Box<Request>, _resp: &mut Box<Response>) -> Result<(), &'static str> { Ok(()) }
}

impl Clone for EmptyContext {
    #[inline]
    fn clone(&self) -> Self { EmptyContext {} }
}
