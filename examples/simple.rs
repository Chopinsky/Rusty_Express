extern crate rusty_express;

use rusty_express::HttpServer;
use rusty_express::http::*;
use rusty_express::router::*;

fn main() {
    let mut server = HttpServer::new(4);

    //delegated definition
    server.get(RequestPath::Exact("/"), Model::simple_response);

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

    pub fn simple_response(req: Request, resp: &mut Response) {
        resp.send(String::from("Hello world from rusty server!\n"));
        resp.status(200);
    }
}

