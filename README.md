# Rusty_Express
A simple http server written in Rust and provide Express-alike APIs.

## How to use
In your project's `Cargo.toml`, add dependency:
```rust
[dependencies]
rusty_express = "0.2.1"
...
```

```rust
extern crate rusty_express;

use rusty_express::HttpServer;
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
