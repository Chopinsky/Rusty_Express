extern crate rust_simple_server;

use rust_simple_server::HttpServer;
use rust_simple_server::http::*;
use rust_simple_server::router::*;

fn main() {
    let mut server = HttpServer::new(4);
    server.def_context(Some(Model::new()));

    //delegated definition
    server.get(RequestPath::Literal("/"), Model::simple_response);

    server.listen(8080);
}

struct Model {
    pub data: i32
}

impl Model  {
    pub fn new() -> Self {
        Model { data: 1 }
    }

    pub fn set_data(&mut self, val: i32) {
       self.data = val;
    }

    pub fn simple_response(path: String, _req: Request, context: Option<Model>) -> String {
        let data = match context {
            Some(val) => val.data,
            None => 0,
        };

        return format!("Hello world from {} with {}!", path, data);
    }
}

