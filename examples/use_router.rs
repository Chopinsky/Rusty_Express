extern crate rusty_express;

use rusty_express::prelude::*;

fn main() {

    // define http server now
    let mut server = HttpServer::new();

    // working with the generic data model
    let mut model = Model::new(100);

    // Define router separately
    let mut router = Route::new();

    router.get(RequestPath::Explicit("/"), Model::simple_response);
    router.get(RequestPath::Explicit("/index"), Model::simple_index);

    server.def_router(router);

    //server.listen(8080);
    server.listen_and_manage(8080, model);
}

struct Model {
    pub count: i32
}

impl Model  {
    pub fn new(d: i32) -> Self {
        Model { count: d }
    }

    pub fn set_data(&mut self, val: i32) {
        self.count = val;
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
            count: self.count.clone(),
        }
    }
}

impl StatesProvider for Model {
    fn interaction_stage(&self) -> StatesInteraction {
        StatesInteraction::WithRequest
    }
}