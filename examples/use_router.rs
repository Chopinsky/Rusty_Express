#![allow(unused_variables)]
#![allow(dead_code)]

extern crate rusty_express;

use rusty_express::prelude::*;
use std::thread;
use std::time::Duration;

fn main() {
    // define http server now
    let mut server = HttpServer::new();

    // working with the generic data model
    let model = Model::new();

    // then delegate the model management to the ServerContext
    ServerContext::set_context(Box::new(model));

    // Define router separately
    let mut router = Route::new();
    router
        .get(RequestPath::Explicit("/"), Model::simple_response)
        .get(RequestPath::Explicit("/index"), Model::simple_index);

    server.def_router(router);

    //server.listen(8080);
    server.listen_and_serve(
        8080,
        Some(|sender| {
            // automatically shutting down after 60 seconds
            thread::sleep(Duration::from_secs(60));

            if let Err(_) = sender.send(ControlMessage::Terminate) {
                eprintln!("Failed to send the server shutdown message...");
            }
        }),
    );
}

struct Model {
    count: u32,
}

impl Model {
    pub fn simple_response(req: &Box<Request>, resp: &mut Box<Response>) {
        Model::work_with_context(req, resp);

        resp.send("Hello world from rusty server!\n");
        resp.status(200);
    }

    pub fn simple_index(req: &Box<Request>, resp: &mut Box<Response>) {
        Model::work_with_context(req, resp);

        resp.send("Hello world from the index page!\n");
        // the status 200 is inferred
    }

    #[inline]
    pub fn new() -> Self {
        Model { count: 0 }
    }

    #[inline]
    pub fn new_with(d: u32) -> Self {
        Model { count: d }
    }

    #[inline]
    fn add_one(&mut self) {
        self.count += 1;
    }

    #[inline]
    fn get_count(&self) -> u32 {
        self.count
    }

    fn work_with_context(req: &Box<Request>, resp: &mut Box<Response>) {
        if let Err(e) = ServerContext::update_context(req, resp) {
            // Error handling...
            eprintln!("Error on updating the server context: {}", e);
        }

        if let Err(e) = ServerContext::process_with_context(req, resp) {
            // Error handling...
            eprintln!("Error on updating the server context: {}", e);
        }
    }
}

impl Clone for Model {
    fn clone(&self) -> Self {
        Model { count: self.count }
    }
}

impl ContextProvider for Model {
    fn update(&mut self, req: &Box<Request>, resp: &mut Box<Response>) -> Result<(), &'static str> {
        self.add_one();
        Ok(())
    }

    fn process(&self, _req: &Box<Request>, _resp: &mut Box<Response>) -> Result<(), &'static str> {
        println!("Visit count: {}", self.count);
        Ok(())
    }
}
