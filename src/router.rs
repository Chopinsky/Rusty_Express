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

impl Route {
    pub fn new() -> Route {
        Route {
            get: HashMap::new(),
            post: HashMap::new(),
            put: HashMap::new(),
            delete: HashMap::new(),
        }
    }


}
