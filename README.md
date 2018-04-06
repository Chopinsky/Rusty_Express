[Rusty_Express][docsrs]
======================

[![Rusty_Express on crates.io][cratesio-image]][cratesio]
[![Rusty_Express on docs.rs][docsrs-image]][docsrs]

[cratesio]: https://crates.io/crates/rusty_express
[cratesio-image]: https://img.shields.io/crates/v/rusty_express.svg
[docsrs-image]: https://docs.rs/rusty_express/badge.svg
[docsrs]: https://docs.rs/rusty_express

## What is this
A simple http server library written in Rust and provide Express-alike APIs. We know that Rust is
hard and daunting, so we will make sure your server can be easy to use without fear!

Version 0.3.0+ is a major milestone, from this point on the APIs shall be mostly stable, and we
expect to make less, if none, break changes, but please do let us know if you've come across bugs
that we should fix, or have met performance bottle necks that we shall try to improve.

## Migrating from 0.2.x to 0.3.0 
0.2.x versions are good experiments with this project. But we're growing fast with better
features and more performance enhancement! That's why we need to start the 0.3.x versions
with slight changes to the interface APIs. 

Here're what to expect when updating from 0.2.x to 0.3.0:
- The route handler function's signature has changed, now the request and response objects
are boxed! So now your route handler should have something similar to this:
```rust
pub fn handler(req: &Box<Request>, resp: &mut Box<Response>) {
    /// work hard to generate the response here...
}
```
- 
  

## How to use
In your project's `Cargo.toml`, add dependency:
```cargo
[dependencies]
rusty_express = "^0.3.0"
...
```

In `src\main.rs`:
```rust
extern crate rusty_express;

use rusty_express::prelude::*;

fn main() {
    //A http server with default thread pool size of 4
    let mut server = HttpServer::new();
    
    //Change thread pool size from 8 (default) to 10.
    server.set_pool_size(10);

    //Route definition
    server.get(RequestPath::Exact("/"), handler);

    //Listen to port 8080, server has started.
    server.listen(8080);
}

pub fn handler(req: &Box<Request>, resp: &mut Box<Response>) {
    resp.send("Hello world from the rusty-express server!\n");
    resp.status(200);
}
```

## Examples
- [Simple server](https://github.com/Chopinsky/Rusty_Express/blob/master/examples/simple.rs)
- [Server with defined router](https://github.com/Chopinsky/Rusty_Express/blob/master/examples/use_router.rs)
- [Use redirect in the router](https://github.com/Chopinsky/Rusty_Express/blob/master/examples/simple_redirect.rs)
