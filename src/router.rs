#![allow(dead_code)]
#![allow(unused_mut)]

use std::collections::HashMap;
use std::cmp::Ordering;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use regex::Regex;
use http::*;

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
    pub params: Vec<String>,
}

impl RegexRoute {
    pub fn new(re: Regex, handler: Callback) -> Self {
        RegexRoute {
            regex: re,
            handler,
            params: Vec::new(),
        }
    }
}

impl Clone for RegexRoute {
    fn clone(&self) -> Self {
        RegexRoute {
            regex: self.regex.clone(),
            handler: self.handler,
            params: self.params.clone(),
        }
    }
}

pub struct RouteMap {
    explicit: HashMap<String, Callback>,
    explicit_with_params: HashMap<String, RegexRoute>,
    wildcard: HashMap<String, RegexRoute>,
}

impl RouteMap {
    pub fn new() -> Self {
        RouteMap {
            explicit: HashMap::new(),
            explicit_with_params: HashMap::new(),
            wildcard: HashMap::new(),
        }
    }

    pub fn insert(&mut self, uri: RequestPath, callback: Callback) {
        match uri {
            RequestPath::Explicit(req_uri) => {
                if req_uri.is_empty() || !req_uri.starts_with("/") {
                    panic!("Request path must have valid contents and start with '/'.");
                }

                self.explicit.entry(req_uri.to_owned()).or_insert(callback);
            },
            RequestPath::WildCard(req_uri) => {
                if req_uri.is_empty() {
                    panic!("Request path must have valid contents.");
                }

                if let Ok(re) = Regex::new(req_uri) {
                    let route = RegexRoute::new(re, callback);
                    self.wildcard.entry(req_uri.to_owned()).or_insert(route);
                }
            },
            RequestPath::ExplicitWithParams(req_uri) => {
                let path: Vec<&str> = req_uri.trim_matches('/').split('/').collect();
                if !req_uri.starts_with('/') && path.is_empty() {
                    panic!("Request path must have valid contents.");
                }

                //TODO: recompile to set params and the regex
                //self.explicit_with_params.push((path, callback));
            },
        }
    }

    fn seek_path(&self, uri: String) -> Option<Callback> {
        if let Some(callback) = self.explicit.get(&uri) {
            return Some(*callback);
        }

        let (tx, rx) = mpsc::channel();

        let dest_path = uri.to_owned();
        let wildcard_router = self.wildcard.to_owned();

        thread::spawn(move || {
            search_wildcard_router(&wildcard_router, dest_path,tx);
        });

        if let Ok(callback) = rx.recv_timeout(Duration::from_millis(200)) {
            match callback {
                None => { return None; },
                _ => { return callback; }
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
    fn handle_request_method(&self, req: &Request, resp: &mut Response);
}

impl RouteHandler for Route {
    fn handle_request_method(&self, req: &Request, resp: &mut Response) {
        let method: REST;
        match req.method {
            None => {
                resp.status(404);
                return;
            },
            Some(ref m) => {
                if m.eq(&REST::OTHER(String::from("head"))) {
                    method = REST::GET;
                } else {
                    method = m.to_owned();
                }
            },
        }

        let uri = req.uri.to_owned();
        if let Some(routes) = self.store.get(&method) {
            handle_request_worker(&routes, &req, resp, uri);
        } else {
            resp.status(404);
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

            handle_request_worker(&routes, &req, resp, redirect.clone());

            resp.header("Location", &redirect, true);
            resp.status(301);
        }
    } else {
        resp.status(404);
    }
}

fn search_wildcard_router(router: &HashMap<String, RegexRoute>, uri: String, tx: mpsc::Sender<Option<Callback>>) {
    let mut result = None;
    for (_, route) in router.iter() {
        if route.regex.is_match(&uri) {
            result = Some(route.handler);
            break;
        }
    }

    match tx.send(result) { _ => { drop(tx); }}
}

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