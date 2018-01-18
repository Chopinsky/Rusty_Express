extern crate rust_simple_server;

use rust_simple_server::HttpServer;

fn main() {
    let mut server = HttpServer::new(4);

    server.route.get(String::from("/"), simple_response);

    server.listen(String::from("8080"));
}

fn simple_response(_path: String) -> String {
    return String::from("Hello world!");
}
