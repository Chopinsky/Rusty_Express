#![allow(dead_code)]
#![allow(unused_mut)]

use std::collections::HashMap;
use regex::Regex;
use http::*;

#[derive(PartialEq, Eq, Hash, Clone)]
pub enum REST {
    NONE,
    GET,
    POST,
    PUT,
    DELETE,
    OPTIONS,
    OTHER(String),
}

#[derive(PartialEq, Eq, Hash, Clone, Copy)]
pub enum RequestPath {
    Explicit(&'static str),
    WildCard(&'static str),
}

pub type Callback = fn(&Request, &mut Response);

pub struct RouteMap {
    explicit: HashMap<String, Callback>,
    wildcard: HashMap<String, Callback>,
}

impl RouteMap {
    pub fn new() -> Self {
        RouteMap {
            explicit: HashMap::new(),
            wildcard: HashMap::new(),
        }
    }

    pub fn insert(&mut self, uri: RequestPath, callback: Callback) {
        match uri {
            RequestPath::Explicit(req_uri) => {
                self.explicit.entry(req_uri.to_owned()).or_insert(callback);
            },
            RequestPath::WildCard(req_uri) => {
                self.wildcard.entry(req_uri.to_owned()).or_insert(callback);
            },
        }
    }

    fn seek_path(&self, uri: String) -> Option<&Callback> {
        if let Some(callback) = self.explicit.get(&uri) {
            return Some(callback);
        }

        for (req_path, callback) in self.wildcard.iter() {
            if let Ok(re) = Regex::new(req_path) {
                if re.is_match(&uri) {
                    return Some(callback);
                }
            }
        }

        None
    }
}

impl Clone for RouteMap {
    fn clone(&self) -> Self {
        RouteMap {
            explicit: self.explicit.clone(),
            wildcard: self.wildcard.clone(),
        }
    }
}

pub struct Route {
    store: HashMap<REST, RouteMap>,
}

impl Route {
    pub fn new() -> Self {
        Route {
            store: HashMap::new(),
        }
    }

    pub fn from(source: &Route) -> Self {
        Route {
            store: source.store.clone(),
        }
    }

    fn add_route(&mut self, method: REST, uri: RequestPath, callback: Callback) {
        if method == REST::NONE { return; }

        if let Some(route) = self.store.get_mut(&method) {
            //find, insert, done.
            route.insert(uri, callback);
            return;
        }

        // the route for the given method has not yet initialized
        let mut route = RouteMap::new();
        route.insert(uri, callback);

        self.store.insert(method, route);
    }
}

pub trait Router {
    fn get(&mut self, uri: RequestPath, callback: Callback);
    fn post(&mut self, uri: RequestPath, callback: Callback);
    fn put(&mut self, uri: RequestPath, callback: Callback);
    fn delete(&mut self, uri: RequestPath, callback: Callback);
    fn options(&mut self, uri: RequestPath, callback: Callback);
    fn other(&mut self, method: &str, uri: RequestPath, callback: Callback);
}

impl Router for Route {
    fn get(&mut self, uri: RequestPath, callback: Callback) {
        self.add_route(REST::GET, uri, callback);
    }

    fn post(&mut self, uri: RequestPath, callback: Callback) {
        self.add_route(REST::POST, uri, callback);
    }

    fn put(&mut self, uri: RequestPath, callback: Callback) {
        self.add_route(REST::PUT, uri, callback);
    }

    fn delete(&mut self, uri: RequestPath, callback: Callback) {
        self.add_route(REST::DELETE, uri, callback);
    }

    fn options(&mut self, uri: RequestPath, callback: Callback) {
        self.add_route(REST::OPTIONS, uri, callback);
    }

    fn other(&mut self, method: &str, uri: RequestPath, callback: Callback) {
        if method.to_lowercase().eq(&"head"[..]) {
            panic!("Can't...");
        }

        let request_method = REST::OTHER(method.to_lowercase().to_owned());
        self.add_route(request_method, uri, callback);
    }
}

pub trait RouteHandler {
    fn handle_request_method(&self, req: &Request, resp: &mut Response);
}

impl RouteHandler for Route {
    fn handle_request_method(&self, req: &Request, resp: &mut Response) {
        if req.method == REST::NONE {
            resp.status(404);
            return;
        } else {
            let uri = req.uri.clone();
            let method =
                if req.method.eq(&REST::OTHER(String::from("head"))) {
                    REST::GET
                } else {
                    req.method.clone()
                };

            if let Some(routes) = self.store.get(&method) {
                handle_request_worker(&routes, &req, resp, uri);
            } else {
                resp.status(404);
            }
        }
    }
}

fn handle_request_worker(routes: &RouteMap, req: &Request, resp: &mut Response, dest: String) {
    if let Some(callback) = routes.seek_path(dest) {
        //Callback function will decide what to be written into the response
        callback(req, resp);

        let mut redirect = resp.get_redirect_path();
        if !redirect.is_empty() {
            resp.redirect("");
            if !redirect.starts_with("/") { redirect.insert(0, '/'); }

            //println!("{}", redirect);

            handle_request_worker(&routes, &req, resp, redirect.clone());

            resp.header("Location", &redirect, true);
            resp.status(301);
        }
    } else {
        resp.status(404);
    }
}