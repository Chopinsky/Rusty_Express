extern crate rusty_express;

use rusty_express::prelude::*;

fn main() {

    // define http server now
    let mut server = HttpServer::new();

    // working with the generic data model
    let mut model = Model::new();
    model.set_data(100);
    server.manage("default", model);

    // Define router separately
    let mut router = Route::new();

    router.get(RequestPath::Explicit("/"), Model::simple_response);
    router.get(RequestPath::Explicit("/index"), Model::simple_index);

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

    pub fn simple_response(_req: &Request, resp: &mut Response) {
        resp.send("Hello world from rusty server!\n");
        resp.status(200);
    }

    pub fn simple_index(_req: &Request, resp: &mut Response) {
        resp.send("Hello world from the index page!\n");
    }
}

impl Clone for Model {
    fn clone(&self) -> Self {
        Model {
            data: self.data.clone(),
        }
    }
}