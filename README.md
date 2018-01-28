# Rusty_Express
A simple http server written in Rust and provide Express-alike APIs.

## How to use

```rust
extern crate rusty_express;

use rusty_express::HttpServer;
use rusty_express::http::*;
use rusty_express::router::*;

fn main() {
    //A http server with thread pool size of 8
    let mut server = HttpServer::new(8);

    //Route definition
    server.get(RequestPath::Exact("/"), simple_response);

    //Listen to port 8080
    server.listen(8080);
}

pub fn simple_response(req: Request, resp: &mut Response) {
    resp.send(String::from("Hello world from the rusty-express server!\n"));
    resp.status(200);
}
```
