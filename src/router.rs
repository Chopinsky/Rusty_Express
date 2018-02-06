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
                self.explicit.entry(req_uri.to_owned()).or_insert(callback);
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
    get: RouteMap,
    post: RouteMap,
    put: RouteMap,
    delete: RouteMap,
    others: RouteMap,
}

pub trait Router {
    fn get(&mut self, uri: RequestPath, callback: Callback);
    fn post(&mut self, uri: RequestPath, callback: Callback);
    fn put(&mut self, uri: RequestPath, callback: Callback);
    fn delete(&mut self, uri: RequestPath, callback: Callback);
    fn other(&mut self, uri: RequestPath, callback: Callback);
}

pub trait RouteHandler {
    fn handle_get(&self, req: Request, resp: &mut Response);
    fn handle_put(&self, req: Request, resp: &mut Response);
    fn handle_post(&self, req: Request, resp: &mut Response);
    fn handle_delete(&self, req: Request, resp: &mut Response);
    fn handle_other(&self, req: Request, resp: &mut Response);
}

impl Route {
    pub fn new() -> Self {
        Route {
            get: RouteMap::new(),
            post: RouteMap::new(),
            put: RouteMap::new(),
            delete: RouteMap::new(),
            others: RouteMap::new(),
        }
    }

    pub fn from(source: &Route) -> Self {
        Route {
            get: source.get.clone(),
            put: source.put.clone(),
            post: source.post.clone(),
            delete: source.delete.clone(),
            others: source.others.clone(),
        }
    }
}

impl Router for Route {
    fn get(&mut self, uri: RequestPath, callback: Callback) {
        self.get.insert(uri, callback);
    }

    fn post(&mut self, uri: RequestPath, callback: Callback) {
        self.post.insert(uri, callback);
    }

    fn put(&mut self, uri: RequestPath, callback: Callback) {
        self.put.insert(uri, callback);
    }

    fn delete(&mut self, uri: RequestPath, callback: Callback) {
        self.delete.insert(uri, callback);
    }

    fn other(&mut self, uri: RequestPath, callback: Callback) {
        self.others.insert(uri, callback);
    }
}

impl RouteHandler for Route {
    fn handle_get(&self, req: Request, resp: &mut Response) {
        let uri = req.uri.clone();
        handle_request_worker(&self.get, &req, resp, uri)
    }

    fn handle_put(&self, req: Request, resp: &mut Response) {
        let uri = req.uri.clone();
        handle_request_worker(&self.put, &req, resp, uri)
    }

    fn handle_post(&self, req: Request, resp: &mut Response) {
        let uri = req.uri.clone();
        handle_request_worker(&self.post, &req, resp, uri)
    }

    fn handle_delete(&self, req: Request, resp: &mut Response) {
        let uri = req.uri.clone();
        handle_request_worker(&self.delete, &req, resp, uri)
    }

    fn handle_other(&self, req: Request, resp: &mut Response) {
        let uri = req.uri.clone();
        handle_request_worker(&self.others, &req, resp, uri)
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