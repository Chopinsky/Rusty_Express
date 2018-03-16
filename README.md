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

## New versions: 0.3.x vs. 0.2.x 
0.2.x versions are good experiments with this project. But we're growing fast with better
features and more performance enhancement! That's why we need to start the 0.3.x versions
with slight changes to the interface APIs. 

Here're what to expect when updating from 0.2.x to 0.3.0:
- sss
- ddd
  

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
