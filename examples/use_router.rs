extern crate rusty_express;

use rusty_express::prelude::*;

fn main() {
    let mut server = HttpServer::new();

    //Define router separately
    let mut router = Route::new();
    router.get(RequestPath::Exact("/"), Model::simple_response);

    server.def_router(router);
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