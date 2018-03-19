#![allow(unused_imports)]
#![allow(unused_variables)]

use std::collections::{HashMap, HashSet};
use std::cmp::Ordering;
use std::io::Error;
use std::sync::mpsc;
use std::time::Duration;

use core::http::{Request, RequestWriter, Response, ResponseStates, ResponseWriter};
use regex::Regex;
use support::debug;
use support::common::MapUpdates;
use support::TaskType;
use support::{RouteTrie, shared_pool};

#[derive(PartialEq, Eq, Hash, Clone)]
pub enum REST {
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
    ExplicitWithParams(&'static str),
    WildCard(&'static str),
}

pub type Callback = fn(&Box<Request>, &mut Box<Response>);

struct RegexRoute {
    pub regex: Regex,
    pub handler: Callback,
}

impl RegexRoute {
    pub fn new(re: Regex, handler: Callback) -> Self {
        RegexRoute {
            regex: re,
            handler,
        }
    }
}

impl Clone for RegexRoute {
    fn clone(&self) -> Self {
        RegexRoute {
            regex: self.regex.clone(),
            handler: self.handler,
        }
    }
}

pub struct RouteMap {
    explicit: HashMap<String, Callback>,
    explicit_with_params: RouteTrie,
    wildcard: HashMap<String, RegexRoute>,
}

impl RouteMap {
    pub fn new() -> Self {
        RouteMap {
            explicit: HashMap::new(),
            explicit_with_params: RouteTrie::initialize(),
            wildcard: HashMap::new(),
        }
    }

    pub fn insert(&mut self, uri: RequestPath, callback: Callback) {
        match uri {
            RequestPath::Explicit(req_uri) => {
                if req_uri.is_empty() || !req_uri.starts_with('/') {
                    panic!("Request path must have valid contents and start with '/'.");
                }

                self.explicit.add(req_uri, callback, false);
            },
            RequestPath::WildCard(req_uri) => {
                if req_uri.is_empty() {
                    panic!("Request path must have valid contents.");
                }

                if self.wildcard.contains_key(req_uri) { return; }

                if let Ok(re) = Regex::new(req_uri) {
                    self.wildcard.add(req_uri, RegexRoute::new(re, callback), false);
                }
            },
            RequestPath::ExplicitWithParams(req_uri) => {
                if !req_uri.contains("/:") {
                    self.explicit.add(req_uri, callback, false);
                    return;
                }

                let segments: Vec<String> = req_uri.trim_matches('/')
                                                   .split('/')
                                                   .filter(|s| !s.is_empty())
                                                   .map(|s| s.to_owned())
                                                   .collect();

                if let Err(e) = validate_segments(&segments) {
                    panic!("{}", e);
                }

                self.explicit_with_params.add(segments, callback);
            },
        }
    }

    fn seek_path(&self, uri: &str, params: &mut HashMap<String, String>) -> Option<Callback> {
        if let Some(callback) = self.explicit.get(uri) {
            return Some(*callback);
        }

        if !self.explicit_with_params.is_empty() {
            let (callback, temp_params) =
                search_params_router(&self.explicit_with_params, uri);

            if callback.is_some() {
                for param in temp_params {
                    params.insert(param.0, param.1);
                }

                return callback;
            }
        }

        if !self.wildcard.is_empty() {
            return search_wildcard_router(&self.wildcard, uri);
        }

        None
    }
}

impl Clone for RouteMap {
    fn clone(&self) -> Self {
        RouteMap {
            explicit: self.explicit.clone(),
            explicit_with_params: self.explicit_with_params.clone(),
            wildcard: self.wildcard.clone(),
        }
    }
}

pub struct Route {
    store: Box<HashMap<REST, RouteMap>>,
}

impl Route {
    pub fn new() -> Self {
        Route {
            store: Box::from(HashMap::new()),
        }
    }

    fn add_route(&mut self, method: REST, uri: RequestPath, callback: Callback) {
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

impl Clone for Route {
    fn clone(&self) -> Self {
        Route {
            store: self.store.clone(),
        }
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
        if method.is_empty() {
            panic!("Must provide a valid method!");
        }

        let request_method = REST::OTHER(method.to_lowercase());
        self.add_route(request_method, uri, callback);
    }
}

pub trait RouteHandler {
    fn handle_request(callback: Callback, req: &mut Box<Request>, resp: &mut Box<Response>);
    fn seek_handler(&self, method: &REST, uri: &str, header_only: bool, tx: mpsc::Sender<(Option<Callback>, HashMap<String, String>)>);
}

impl RouteHandler for Route {
    fn handle_request(callback: Callback, req: &mut Box<Request>, resp: &mut Box<Response>) {
        handle_request_worker(&callback, &req, resp);
    }

    fn seek_handler(&self, method: &REST, uri: &str, header_only: bool,
                    tx: mpsc::Sender<(Option<Callback>, HashMap<String, String>)>) {

        let mut result = None;
        let mut params = HashMap::new();

        if let Some(routes) = self.store.get(method) {
            result = routes.seek_path(uri, &mut params);
        } else if header_only {
            //if a header only request, fallback to search with REST::GET
            if let Some(routes) = self.store.get(&REST::GET) {
                result = routes.seek_path(uri, &mut params);
            }
        }

        if let Err(e) = tx.send((result, params)) {
            debug::print("Unable to find the route handler", 2);
        }
    }
}

fn handle_request_worker(callback: &Callback, req: &Box<Request>, resp: &mut Box<Response>) {
    // callback function will decide what to be written into the response
    callback(req, resp);

    // if a redirect response, set up as so.
    let mut redirect = resp.get_redirect_path();
    if !redirect.is_empty() {
        if !redirect.starts_with('/') { redirect.insert(0, '/'); }

        resp.header("Location", &redirect, true);
        resp.status(301);
    }
}

fn search_wildcard_router(routes: &HashMap<String, RegexRoute>, uri: &str) -> Option<Callback> {
    let mut result = None;
    for (_, route) in routes.iter() {
        if route.regex.is_match(&uri) {
            result = Some(route.handler);
            break;
        }
    }

    result
}

fn search_params_router(route_head: &RouteTrie, uri: &str) -> (Option<Callback>, Vec<(String, String)>) {
    let raw_segments: Vec<String> = uri.trim_matches('/').split('/').map(|s| s.to_owned()).collect();
    let segements = raw_segments.as_slice();
    let mut params: Vec<(String, String)> = Vec::new();

    let result =
        RouteTrie::find(&route_head.root, segements, &mut params);

    (result, params)
}

fn validate_segments(segments: &Vec<String>) -> Result<(), &'static str> {
    let mut param_names = HashSet::new();
    for seg in segments {
        if seg.starts_with(':') {
            if param_names.contains(seg) {
                return Err("Route parameters must have unique names.");
            } else {
                param_names.insert(seg);
            }
        }
    }

    Ok(())
}