[Rusty_Express][docsrs] v0.2.2
======================

[![Rusty_Express on crates.io][cratesio-image]][cratesio]
[![Rusty_Express on docs.rs][docsrs-image]][docsrs]

[cratesio]: https://crates.io/crates/rusty_express
[cratesio-image]: https://img.shields.io/crates/v/rusty_express.svg
[docsrs-image]: https://docs.rs/rusty_express/badge.svg?version=0.2.2
[docsrs]: https://docs.rs/rusty_express/0.2.2/rusty_express/

## What is this
A simple http server library written in Rust and provide Express-alike APIs.

## Under Development
_This library is still actively worked on, to get the latest feature and bug fixes, please update to the latest version._

## How to use
In your project's `Cargo.toml`, add dependency:
```rust
[dependencies]
rusty_express = "^0.2.2"
...
```

In `src\main.rs`:
```rust
extern crate rusty_express;

use rusty_express::HttpServer;
use rusty_express::ServerDef;
use rusty_express::http::*;
use rusty_express::router::*;

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

pub fn simple_response(req: Request, resp: &mut Response) {
    resp.send(String::from("Hello world from the rusty-express server!\n"));
    resp.status(200);
}
```
