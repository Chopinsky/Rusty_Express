#![allow(unused_variables)]

extern crate rusty_express;

use rusty_express::prelude::*;

fn main() {
    let mut server = HttpServer::new();
    server.set_pool_size(8);

    //define router directly
    server.get(RequestPath::WildCard(r"/\w+"), simple_response);

    server.listen(8080);
}

pub fn simple_response(req: &Request, resp: &mut Response) {
    resp.send(&format!("Hello world from rusty server from {}!\n", req.uri));
    resp.status(200);
}

