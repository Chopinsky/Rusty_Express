#![allow(unused_variables)]
#![allow(dead_code)]

extern crate rusty_express;

use std::sync::{Arc, RwLock};
use std::thread;
use rusty_express::prelude::*;

//lazy_static! {
//    static ref COUNT: RwLock<u64> = RwLock::new(0);
//}

fn main() {
    // define http server now
    let mut server = HttpServer::new();

    // working with the generic data model
    let model = Model::new(100);

    // Define router separately
    let mut router = Route::new();
    router.get(RequestPath::Explicit("/"), Model::simple_response);
    router.get(RequestPath::Explicit("/index"), Model::simple_index);

    server.def_router(router);
    server.listen_and_manage(8080, Arc::new(RwLock::new(model)));
}

struct Model {
    count: i32
}

impl Model {
    pub fn new(d: i32) -> Self {
        Model { count: d }
    }

    pub fn simple_response(_req: &Request, resp: &mut Response) {
        resp.send("Hello world from rusty server!\n");
        resp.status(200);
    }

    pub fn simple_index(_req: &Request, resp: &mut Response) {
        resp.send("Hello world from the index page!\n");
    }

    fn get_count(&self) -> i32 {
        self.count
    }

    fn set_count(&mut self, val: i32) {
        self.count = val;
    }
}

impl Clone for Model {
    fn clone(&self) -> Self {
        Model {
            count: self.count,
        }
    }
}

impl StatesProvider for Model {
    fn interaction_stage(&self) -> StatesInteraction {
        StatesInteraction::WithRequest
    }

    fn on_request(&self, _: &mut Request) -> RequireStateUpdates { true }

    fn on_response(&self, _: &mut Response) -> RequireStateUpdates { false }

    fn update(&mut self, req: &Request, resp: Option<&Response>) {
        // Blocking way
        let count = self.count;
        self.set_count(count + 1);

        if let None = resp {
            println!("Visit counts: {}", self.get_count());
        }

        // Non-Blocking way
        thread::spawn(move || {
//            if let Ok(mut count) = COUNT.write() {
//                *count = *count + 1;
//            }
        });
    }
}