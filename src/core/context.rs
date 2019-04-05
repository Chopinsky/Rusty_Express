#![allow(dead_code)]
#![allow(clippy::borrowed_box)]

use super::http::{Request, Response};
use parking_lot::RwLock;

lazy_static! {
    static ref CONTEXT: RwLock<Box<ServerContextProvider>> = RwLock::new(Box::new(EmptyContext {}));
}

pub type ServerContextProvider = ContextProvider + Sync + Send;

pub trait ContextProvider {
    fn update(&mut self, req: &Box<Request>, resp: &mut Box<Response>) -> Result<(), &'static str>;
    fn process(&self, req: &Box<Request>, resp: &mut Box<Response>) -> Result<(), &'static str>;
}

pub fn set_context(context: Box<ServerContextProvider>) {
    let mut c = CONTEXT.write();
    *c = context;
}

pub fn update_context(req: &Box<Request>, resp: &mut Box<Response>) -> Result<(), &'static str> {
    let mut c = CONTEXT.write();
    c.update(req, resp)
}

pub fn process_with_context(
    req: &Box<Request>,
    resp: &mut Box<Response>,
) -> Result<(), &'static str> {
    let c = CONTEXT.read();
    c.process(req, resp)
}

struct EmptyContext;

impl ContextProvider for EmptyContext {
    #[inline]
    fn update(
        &mut self,
        _req: &Box<Request>,
        _resp: &mut Box<Response>,
    ) -> Result<(), &'static str> {
        Ok(())
    }

    #[inline]
    fn process(&self, _req: &Box<Request>, _resp: &mut Box<Response>) -> Result<(), &'static str> {
        Ok(())
    }
}

impl Clone for EmptyContext {
    #[inline]
    fn clone(&self) -> Self {
        EmptyContext {}
    }
}
