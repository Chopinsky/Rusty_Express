[Rusty_Express][docsrs]
======================

[![Rusty_Express on crates.io][cratesio-image]][cratesio]
[![Rusty_Express on docs.rs][docsrs-image]][docsrs]

[cratesio]: https://crates.io/crates/rusty_express
[cratesio-image]: https://img.shields.io/crates/v/rusty_express.svg
[docsrs-image]: https://docs.rs/rusty_express/badge.svg
[docsrs]: https://docs.rs/rusty_express

## What is this
A simple http server library written in Rust and provide Express-alike APIs.

## Moving to version 0.3.0 
Even though there are many things left undone for version 0.2.x, I'm planning on making slight
changes to the interface APIs, which may no longer be compatible with your projects using 0.2.x. 

So I'm going to publish 0.2.9 as the last version of the 0.2.x series. The upcoming 0.3.x series
will be as awesome with slight interface API updates, and hopefully, better documentation and test
coverage!

Wohooo! 
  

## How to use
In your project's `Cargo.toml`, add dependency:
```cargo
[dependencies]
rusty_express = "^0.2.9"
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
    server.get(RequestPath::Exact("/"), simple_response);

    //Listen to port 8080, server has started.
    server.listen(8080);
}

pub fn simple_response(req: &Request, resp: &mut Response) {
    resp.send("Hello world from the rusty-express server!\n");
    resp.status(200);
}
```

## Examples
- [Simple server](https://github.com/Chopinsky/Rusty_Express/blob/master/examples/simple.rs)
- [Server with defined router](https://github.com/Chopinsky/Rusty_Express/blob/master/examples/use_router.rs)
- [Use redirect in the router](https://github.com/Chopinsky/Rusty_Express/blob/master/examples/simple_redirect.rs)
