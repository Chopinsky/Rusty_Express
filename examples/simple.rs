extern crate rust_simple_server;

use std::collections::HashMap;
use rust_simple_server::*;

fn main() {
    let default_server_definition: HttpServerDefinition = HttpServerDefinition {
        threads: 1,
        route: router::Route {
            get: HashMap::new(),
            put: HashMap::new(),
            post: HashMap::new(),
            delete: HashMap::new(),
        },
    };

    BaseServer::start_with(&default_server_definition);
}