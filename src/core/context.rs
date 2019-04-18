#![allow(dead_code)]
#![allow(clippy::borrowed_box)]

use crate::core::http::{Request, Response};
use parking_lot::RwLock;

const ERR_STR: &str = "The context has not been initialized...";
static mut CONTEXT: Option<RwLock<Box<ServerContextProvider>>> = None;

pub type ServerContextProvider = ContextProvider + Sync + Send;

pub trait ContextProvider {
    fn update(&mut self, req: &Request, resp: &mut Response) -> Result<(), &'static str>;
    fn process(&self, req: &Request, resp: &mut Response) -> Result<(), &'static str>;
}

/// Move the ownership of the context object to the server, such that it can be managed when
/// it receives new requests.
pub fn set_context(context: Box<ServerContextProvider>) {
    if let Some(ctx) = unsafe { CONTEXT.as_mut() } {
        let mut ctx = ctx.write();
        *ctx = context;
        return;
    }

    unsafe { CONTEXT = Some(RwLock::new(context)); }
}

/// Update the context content. This will invoke the `RwLock` to gain the exclusive access
/// to the underlying context object, hence impacting the performance. You don't need to lock
/// the object again, unless it's also shared with other threads not interacting with the TCP
/// stream.
///
/// For examples, see [`https://github.com/Chopinsky/Rusty_Express/blob/master/examples/use_router.rs`]
pub fn update_context(
    req: &Request,
    resp: &mut Response
) -> Result<(), &'static str>
{
    if let Some(ctx) = unsafe { CONTEXT.as_mut() } {
        let mut ctx = ctx.write();
        return ctx.update(req, resp);
    }

    Err(ERR_STR)
}

/// Access the context content. This will invoke the read lock of the `RwLock` to gain the
/// shared access to the underlying context object, the impact to the performance should consider
/// normal. You don't need to lock the object again, unless it's also shared with other threads
/// not interacting with the TCP stream.
///
/// For examples, see [`https://github.com/Chopinsky/Rusty_Express/blob/master/examples/use_router.rs`]
pub fn process_with_context(
    req: &Request,
    resp: &mut Response,
) -> Result<(), &'static str>
{
    if let Some(ctx) = unsafe { CONTEXT.as_ref() } {
        let ctx = ctx.read();
        return ctx.process(req, resp);
    }

    Err(ERR_STR)
}