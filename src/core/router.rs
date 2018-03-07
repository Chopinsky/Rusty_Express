#![allow(unused_imports)]
#![allow(unused_variables)]

use std::collections::HashMap;
use std::cmp::Ordering;
use std::sync::mpsc;
use std::time::Duration;

use core::http::{Request, RequestWriter, Response, ResponseStates, ResponseWriter};
use regex::Regex;
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

pub type Callback = fn(&Request, &mut Response);

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
    explicit_with_params: RouteTrie, //HashMap<String, RegexRoute>,
    wildcard: HashMap<String, RegexRoute>,
}

impl RouteMap {
    pub fn new() -> Self {
        RouteMap {
            explicit: HashMap::new(),
            explicit_with_params: RouteTrie::initialize(), //HashMap::new(),
            wildcard: HashMap::new(),
        }
    }

    pub fn insert(&mut self, uri: RequestPath, callback: Callback) {
        match uri {
            RequestPath::Explicit(req_uri) => {
                if req_uri.is_empty() || !req_uri.starts_with('/') {
                    panic!("Request path must have valid contents and start with '/'.");
                }

                self.explicit.entry(req_uri.to_owned()).or_insert(callback);
            },
            RequestPath::WildCard(req_uri) => {
                if req_uri.is_empty() {
                    panic!("Request path must have valid contents.");
                }

                if self.wildcard.contains_key(req_uri) { return; }

                if let Ok(re) = Regex::new(req_uri) {
                    let route = RegexRoute::new(re, callback);
                    self.wildcard.entry(req_uri.to_owned()).or_insert(route);
                }
            },
            RequestPath::ExplicitWithParams(req_uri) => {
                if !req_uri.contains("/:") {
                    self.explicit.entry(req_uri.to_owned()).or_insert(callback);
                    return;
                }

                let segments: Vec<String> = req_uri.trim_matches('/')
                                                   .split('/')
                                                   .filter(|s| !s.is_empty())
                                                   .map(|s| s.to_owned())
                                                   .collect();

                self.explicit_with_params.add(segments, callback);
            },
        }
    }

    fn seek_path(&self, uri: &str, params: &mut HashMap<String, String>) -> Option<Callback> {
        if let Some(callback) = self.explicit.get(uri) {
            return Some(*callback);
        }

        let (tx, rx) = mpsc::channel();

        if !self.wildcard.is_empty() {
            let wildcard_routes = self.wildcard.to_owned();
            let dest_path = uri.to_owned();
            let tx_clone = mpsc::Sender::clone(&tx);

            shared_pool::run(move || {
                search_wildcard_router(&wildcard_routes, dest_path, tx_clone);
            });
        }

        if !self.explicit_with_params.is_empty() {
            let route_head = self.explicit_with_params.to_owned();
            let dest_path = uri.to_owned();
            let tx_clone = mpsc::Sender::clone(&tx);

            shared_pool::run(move || {
                search_params_router(&route_head, dest_path, tx_clone);
            });
        }

        drop(tx);
        for results in rx {
            if let Some(callback) = results.0 {
                for param in results.1 {
                    params.insert(param.0, param.1);
                }

                return Some(callback);
            }
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
        if method.to_lowercase().eq(&"head"[..]) {
            panic!("Can't...");
        }

        let request_method = REST::OTHER(method.to_lowercase().to_owned());
        self.add_route(request_method, uri, callback);
    }
}

pub trait RouteHandler {
    fn handle_request_method(&self, req: &mut Request, resp: &mut Response);
}

impl RouteHandler for Route {
    fn handle_request_method(&self, req: &mut Request, resp: &mut Response) {
        if let Some(ref req_method) = req.method.to_owned() {
            let method = match req_method {
                &REST::OTHER(ref others) => {
                    if others.eq("head") {
                        &REST::GET
                    } else {
                        req_method
                    }
                },
                _ => { req_method },
            };

            if let Some(routes) = self.store.get(method) {
                let mut params = HashMap::new();
                if let Some(callback) = routes.seek_path(&req.uri[..], &mut params) {
                    if !params.is_empty() {
                        req.create_param(params);
                    }

                    handle_request_worker(&callback, req, resp);
                    return;
                }
            }
        }

        resp.status(404);
    }
}

fn handle_request_worker(callback: &Callback, req: &Request, resp: &mut Response) {
    //Callback function will decide what to be written into the response
    callback(req, resp);

    let mut redirect = resp.get_redirect_path();
    if !redirect.is_empty() {
        if !redirect.starts_with('/') { redirect.insert(0, '/'); }

        //TODO: Never provide content directly?? Then move line below...
        //resp.redirect("");
        //handle_request_worker(&routes, &req, resp, redirect.clone());

        resp.header("Location", &redirect, true);
        resp.status(301);
    }
}

fn search_wildcard_router(routes: &HashMap<String, RegexRoute>, uri: String, tx: mpsc::Sender<(Option<Callback>, Vec<(String, String)>)>) {
    let mut result = None;
    for (_, route) in routes.iter() {
        if route.regex.is_match(&uri) {
            result = Some(route.handler);
            break;
        }
    }

    tx.send((result, Vec::with_capacity(0))).unwrap_or_else(|e| {
        eprintln!("Error on matching wild card routes: {}", e);
    });
}

fn search_params_router(route_head: &RouteTrie, uri: String, tx: mpsc::Sender<(Option<Callback>, Vec<(String, String)>)>) {
    let raw_segments: Vec<String> = uri.trim_matches('/').split('/').map(|s| s.to_owned()).collect();
    let segements = raw_segments.as_slice();
    let mut params: Vec<(String, String)> = Vec::new();

    let result = RouteTrie::find(&route_head.root, segements, &mut params);

    tx.send((result, params)).unwrap_or_else(|e| {
        eprintln!("Error on matching wild card routes: {}", e);
    });
}

/*
struct RouteMatchWithParam {
    handler: Callback,
    params: HashMap<String, String>,
}

impl RouteMatchWithParam {
    pub fn new(handler: Callback, params: HashMap<String, String>) -> Self {
        RouteMatchWithParam {
            handler,
            params,
        }
    }
}

fn search_params_router(router: &HashMap<String, RegexRoute>, uri: String, tx: mpsc::Sender<Option<Callback>>) {

}

fn search_params_router2(router: &Vec<(Vec<&str>, Callback)>, uri: String, tx: mpsc::Sender<Option<Callback>>) {
    let path: Vec<&str> = uri.trim_matches('/').split('/').collect();
    if path.is_empty() {
        match tx.send(None) { _ => { drop(tx); }}
        return;
    }

    let path_len = path.len();
    for &ref pair in router.iter() {
        //TODO: clear up the params HashMap always, or later?

        if pair.0.len() != path_len { continue; }

        let mut found = false;
        let mut index: usize = 0;

        for node in pair.0.iter() {
            index += 1;
            if let Some(val) = path.get(index-1) {
                if (*node).starts_with("{") && (*node).ends_with("}") {
                    //TODO: add to the params HashMap

                } else if (*node).cmp(val) == Ordering::Equal {
                    if index == path_len {
                        found = true;
                        break;
                    }

                    continue;
                } else {
                    break;
                }
            }
        }

        if found { break; }
    }

    match tx.send(None) { _ => { drop(tx); }}
}
*/