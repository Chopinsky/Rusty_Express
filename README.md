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

## Feature Request and PR Welcome 
The project is new and lack many common http server features. Please feel free to submit feature request as an issue.

You're also very welcome to submit PR to fix bugs or implement new features. 

## How to use
In your project's `Cargo.toml`, add dependency:
```rust
[dependencies]
rusty_express = "^0.2.7"
...
```

In `src\main.rs`:
```rust
extern crate rusty_express;

use rusty_express::prelude::*;

fn main() {
    //A http server with default thread pool size of 4
    let mut server = HttpServer::new();
    //Change thread pool size to 8.
    server.set_pool_size(8);

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
