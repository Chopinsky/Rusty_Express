#![allow(dead_code)]

use std::collections::HashMap;
use http::Request;

pub enum REST {
    NONE,
    GET,
    POST,
    PUT,
    DELETE,
}

impl Default for REST {
    fn default() -> REST { REST::NONE }
}

pub struct Route {
    get: HashMap<String, fn(String, Request) -> String>,
    post: HashMap<String, fn(String, Request) -> String>,
    put: HashMap<String, fn(String, Request) -> String>,
    delete: HashMap<String, fn(String, Request) -> String>,
}

//TODO: trait for Router
pub trait Router {
    fn get(&mut self, uri:String, callback: fn(String, Request) -> String);
    fn post(&mut self, uri:String, callback: fn(String, Request) -> String);
    fn put(&mut self, uri:String, callback: fn(String, Request) -> String);
    fn delete(&mut self, uri:String, callback: fn(String, Request) -> String);
}

//TODO: impl trait for Router
impl Route {
    pub fn new() -> Self {
        Route {
            get: HashMap::new(),
            post: HashMap::new(),
            put: HashMap::new(),
            delete: HashMap::new(),
        }
    }
}

impl Router for Route {
    fn get(&mut self, uri: String, callback: fn(String, Request) -> String) {
        if !uri.is_empty() {
            self.get.insert(uri, callback);
        }
    }

    fn put(&mut self, uri: String, callback: fn(String, Request) -> String) {
        if !uri.is_empty() {
            self.get.insert(uri, callback);
        }
    }

    fn post(&mut self, uri: String, callback: fn(String, Request) -> String) {
        if !uri.is_empty() {
            self.get.insert(uri, callback);
        }
    }

    fn delete(&mut self, uri: String, callback: fn(String, Request) -> String) {
        if !uri.is_empty() {
            self.get.insert(uri, callback);
        }
    }
}
