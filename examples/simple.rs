extern crate rust_simple_server;

use rust_simple_server::HttpServer;
use rust_simple_server::http::*;
use rust_simple_server::router::Router;

fn main() {
    let mut server = HttpServer::new(4);

    //literal definition
    //server.route.get(String::from("/"), simple_response);

    //delegated definition
    server.get(String::from("/"), simple_response);

    server.listen(8080);
}

fn simple_response(path: String, _req: Request) -> String {
    return String::from("Hello world from {}!", path);
}
