#![allow(dead_code)]
#![allow(clippy::borrowed_box)]

use super::http::{Request, Response};
use parking_lot::RwLock;

const ERR_STR: &'static str = "The context has not been initialized...";
static mut C: Option<RwLock<Box<ServerContextProvider>>> = None;

pub type ServerContextProvider = ContextProvider + Sync + Send;

pub trait ContextProvider {
    fn update(&mut self, req: &Request, resp: &mut Response) -> Result<(), &'static str>;
    fn process(&self, req: &Request, resp: &mut Response) -> Result<(), &'static str>;
}

pub fn set_context(context: Box<ServerContextProvider>) {
    if let Some(ctx) = unsafe { C.as_mut() } {
        let mut ctx = ctx.write();
        *ctx = context;
        return;
    }

    unsafe { C = Some(RwLock::new(context)); }
}

pub fn update_context(
    req: &Request,
    resp: &mut Response
) -> Result<(), &'static str>
{
    if let Some(ctx) = unsafe { C.as_mut() } {
        let mut ctx = ctx.write();
        return ctx.update(req, resp);
    }

    Err(ERR_STR)
}

pub fn process_with_context(
    req: &Request,
    resp: &mut Response,
) -> Result<(), &'static str>
{
    if let Some(ctx) = unsafe { C.as_ref() } {
        let ctx = ctx.read();
        return ctx.process(req, resp);
    }

    Err(ERR_STR)
}