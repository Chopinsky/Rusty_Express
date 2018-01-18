#![allow(dead_code)]

use std::collections::HashMap;

pub enum REST {
    NONE,
    GET,
    POST,
    PUT,
    DELETE,
}

pub struct Route {
    get: HashMap<String, fn(String) -> String>,
    post: HashMap<String, fn(String) -> String>,
    put: HashMap<String, fn(String) -> String>,
    delete: HashMap<String, fn(String) -> String>,
}

//TODO: trait for Router

//TODO: impl trait for Router

impl Route {
    pub fn new() -> Route {
        Route {
            get: HashMap::new(),
            post: HashMap::new(),
            put: HashMap::new(),
            delete: HashMap::new(),
        }
    }

    pub fn get(&mut self, uri: String, callback: fn(String) -> String) {
        if !uri.is_empty() {
            self.get.insert(uri, callback);
        }
    }
}
