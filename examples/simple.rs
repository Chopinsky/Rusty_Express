extern crate rust_simple_server;

use rust_simple_server::HttpServer;

fn main() {
    let server = HttpServer::new(4);
    server.listen(String::from("8080"));
}