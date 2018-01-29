#![allow(unused_variables)]

extern crate rusty_express;

use rusty_express::HttpServer;
use rusty_express::http::*;
use rusty_express::router::*;

fn main() {
    let mut server = HttpServer::new();

    //define router directly
    server.get(RequestPath::Exact("/"), simple_response);

    server.listen(8080);
}

pub fn simple_response(req: Request, resp: &mut Response) {
    resp.send(String::from("Hello world from rusty server!\n"));
    resp.status(200);
}

