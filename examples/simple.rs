#![allow(unused_variables)]

extern crate rusty_express;

use rusty_express::prelude::*;

fn main() {
    let mut server = HttpServer::new();
    server.set_pool_size(8);

    //define router directly
    server.get(RequestPath::Exact("/"), simple_response);

    server.listen(8080);
}

pub fn simple_response(req: Request, resp: &mut Response) {
    resp.send(String::from("Hello world from rusty server!\n"));
    resp.status(200);
}

