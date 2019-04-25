#![allow(dead_code)]
#![allow(clippy::borrowed_box)]

use std::fmt;
use std::path::PathBuf;
use std::sync::RwLock;
use std::fs;

use crate::channel;
use crate::core::http::{Request, Response, ResponseWriter};
use crate::hashbrown::{HashMap, HashSet};
use crate::support::common::MapUpdates;
use crate::support::debug::{self, InfoLevel};
use crate::support::Field;
use crate::support::RouteTrie;

use regex::Regex;

lazy_static! {
    static ref ROUTE_FOR_ALL: REST = REST::OTHER(String::from("*"));
    static ref ROUTER: RwLock<Route> = RwLock::new(Route::new());
}

#[derive(PartialEq, Eq, Hash, Clone)]
pub enum REST {
    GET,
    PATCH,
    POST,
    PUT,
    DELETE,
    OPTIONS,
    OTHER(String),
}

impl fmt::Display for REST {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        match self {
            REST::GET => write!(fmt, "GET"),
            REST::PATCH => write!(fmt, "PATCH"),
            REST::POST => write!(fmt, "POST"),
            REST::PUT => write!(fmt, "PUT"),
            REST::DELETE => write!(fmt, "DELETE"),
            REST::OPTIONS => write!(fmt, "OPTIONS"),
            REST::OTHER(s) => write!(fmt, "{}", s),
        }
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Copy)]
pub enum RequestPath {
    Explicit(&'static str),
    ExplicitWithParams(&'static str),
    WildCard(&'static str),
}

/// `Callback` is a type alias to the REST request handler functions, which will be invoked when a
/// client request has been received on the associated URI or pattern.
pub type Callback = fn(&Box<Request>, &mut Box<Response>);

/// `AuthFunc` is a type alias to the authentication functions, which is optional, but if set, it
/// will be invoked right after we parse the client request to determine if the requested URI is
/// allowed to be visited by the client: if denied, we will generate the 403 error message as the
/// response. This function is generally to be used as the gate-keeper, e.g. if a use is logged in
/// to see the dashboard routes.
///
/// The function takes 2 input parameters: 1) request: &Box<Request>, which contains all information
/// from the client request; 2) the URI from the request: String, which is the URI being requested,
/// this information is also available from the `request` parameter, but we extracted out to make it
/// easier to access.
///
/// The function takes 1 boolean output, where `true` means the access is allowed, and `false` means
/// the access is denied.
///
/// The use of the AuthFunc is totally optional, you can also check authentication within individual
/// request handlers as well. You can also use the `context` and/or `session` modules to store, or
/// update persistent information regarding the client requestor.
pub type AuthFunc = fn(&Box<Request>, &String) -> bool;

struct RegexRoute {
    regex: Regex,
    handler: RouteHandler,
}

impl RegexRoute {
    pub(crate) fn new(re: Regex, handler: RouteHandler) -> Self {
        RegexRoute { regex: re, handler }
    }
}

impl Clone for RegexRoute {
    fn clone(&self) -> Self {
        RegexRoute {
            regex: self.regex.clone(),
            handler: self.handler.clone(),
        }
    }
}

struct StaticLocRoute {
    location: PathBuf,
    black_list: HashSet<String>,
    white_list: HashSet<String>,
}

pub(crate) struct RouteMap {
    explicit: HashMap<String, RouteHandler>,
    explicit_with_params: RouteTrie,
    wildcard: HashMap<String, RegexRoute>,
    static_path: Option<StaticLocRoute>,
}

impl RouteMap {
    pub fn new() -> Self {
        RouteMap {
            explicit: HashMap::new(),
            explicit_with_params: RouteTrie::initialize(),
            wildcard: HashMap::new(),
            static_path: None,
        }
    }

    pub fn insert(&mut self, uri: RequestPath, callback: RouteHandler) {
        match uri {
            RequestPath::Explicit(req_uri) => {
                if req_uri.is_empty() || !req_uri.starts_with('/') {
                    panic!("Request path must have valid contents and start with '/'.");
                }

                self.explicit.add(req_uri, callback, false);
            }
            RequestPath::WildCard(req_uri) => {
                if req_uri.is_empty() {
                    panic!("Request path must have valid contents.");
                }

                if self.wildcard.contains_key(req_uri) {
                    return;
                }

                if let Ok(re) = Regex::new(req_uri) {
                    self.wildcard
                        .add(req_uri,
                             RegexRoute::new(re, callback),
                             false
                        );
                }
            }
            RequestPath::ExplicitWithParams(req_uri) => {
                if !req_uri.contains("/:") && !req_uri.contains(":\\") {
                    self.explicit.add(req_uri, callback, false);
                    return;
                }

                self.explicit_with_params
                    .add(RouteMap::params_parser(req_uri), callback.0, callback.1);
            }
        }
    }

    fn params_parser(source_uri: &'static str) -> Vec<Field> {
        let mut param_names = HashSet::new();

        let mut validation: Option<Regex> = None;
        let mut name = "";
        let mut is_param = false;

        // Status: 0 -- Normal;
        //         1 -- Just split;
        //         2 -- In params;
        //         4 -- In params regex;
        //         8 -- Params regex just end, must split next or panic;
        let mut split_status: u8 = 0;

        let mut result: Vec<Field> = source_uri
            .split(|c|
                // split status Automator
                match c {
                    ':' if split_status == 1 => {
                        split_status <<= 1;  // 2 -- in params
                        false
                    },
                    '(' if split_status == 2 => {
                        split_status <<= 1; // 4 -- in params regex
                        false
                    },
                    ')' if split_status == 4 => {
                        split_status <<= 1; // 8 -- in params regex end
                        false
                    },
                    '/' if split_status == 0 || split_status == 2 || split_status == 8 => {
                        split_status = 1;   // reset to 1 -- just split
                        true
                    },
                    '/' if split_status == 1 => {
                        panic!("Route can't contain empty segment between '/'s: {}", source_uri);
                    },
                    _ => {
                        if split_status == 2 && !char::is_alphanumeric(c) {
                            panic!("Route's parameter name can only contain alpha-numeric characters: {}", source_uri);
                        }

                        if split_status == 8 {
                            panic!("Route's parameter with regex validation must end after the regex: {}", source_uri);
                        }

                        if split_status == 1 {
                            split_status >>= 1; // does not encounter special flags, this is an explicit uri segment name
                        }

                        false
                    },
                }
            )
            .filter_map(|s| {
                if s.is_empty() {
                    return None;
                }

                validation = None;
                is_param = false;

                if s.starts_with(':') {
                    name = &s[1..];

                    if name.is_empty() {
                        panic!("Route parameter name can't be null");
                    }

                    is_param = true;
                    if name.len() > 1 && name.ends_with(')') {
                        let name_split: Vec<&str> =
                            (&name[..name.len()-1]).splitn(2, '(').collect();

                        if name_split.len() == 2 {
                            if name_split[0].is_empty() {
                                panic!("Route parameters with regex validation must have a non-null param name: {}", s);
                            } else if name_split[1].is_empty() {
                                panic!("Route parameters with regex validation must have a non-null regex: {}", s);
                            }

                            if let Ok(regex) = Regex::new(name_split[1]) {
                                validation = Some(regex);
                                name = name_split[0];
                            }
                        }
                    }

                    if param_names.contains(name) {
                        panic!("Route parameters must have unique name: {}", s);
                    }

                    param_names.insert(name.to_owned());
                } else {
                    name = &s;
                }

                Some(Field::new(name.to_owned(), is_param, validation.take()))
            })
            .collect();

        result.reverse();
        result
    }

    fn seek_path(&self, raw_uri: &str, params: &mut HashMap<String, String>) -> RouteHandler {
        if raw_uri.is_empty() {
            return RouteHandler::default();
        }

        let mut actual_uri = "/";
        let mut file_name = "";

        raw_uri.rsplitn(2, '/').enumerate().for_each(|(i, x)| {
            if i == 0 && x.contains('.') {
                file_name = x;
            } else {
                if file_name.is_empty() {
                    actual_uri = raw_uri;
                } else if !x.is_empty() {
                    actual_uri = x;
                }
            }
        });

        if let Some(callback) = self.explicit.get(actual_uri) {
            return RouteHandler::update_handler(callback.clone(), file_name);
        }

        if let Some(static_path) = self.static_path.as_ref() {
            match search_static_router(&static_path, raw_uri) {
                Ok(res) => {
                    // if res is some, we're done. Searching with the full sym-link, so we use
                    // the raw uri instead of the /custom/path/to/uri, and we don't want to
                    // append the file name again since we check the file-existence already
                    if res.is_some() {
                        return res;
                    }
                },
                Err(_) => {
                    // either not in white-list, or in black-list, quit
                    return RouteHandler::default();
                },
            }
        }

        if !self.explicit_with_params.is_empty() {
            let result =
                search_params_router(&self.explicit_with_params, actual_uri, params);

            if result.is_some() {
                return RouteHandler::update_handler(result, file_name);
            }
        }

        if !self.wildcard.is_empty() {
            return RouteHandler::update_handler(
                search_wildcard_router(&self.wildcard, actual_uri),
                file_name
            );
        }

        RouteHandler(None, None)
    }
}

impl Clone for RouteMap {
    fn clone(&self) -> Self {
        RouteMap {
            explicit: self.explicit.clone(),
            explicit_with_params: self.explicit_with_params.clone(),
            wildcard: self.wildcard.clone(),
            static_path: None,
        }
    }
}

#[derive(Default)]
pub struct Route {
    store: HashMap<REST, RouteMap>,
    auth_func: Option<AuthFunc>,
}

impl Route {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn get_auth_func() -> Option<AuthFunc> {
        if let Ok(route) = ROUTER.read() {
            return route.auth_func;
        }

        None
    }

    pub fn set_auth_func(auth_func: Option<AuthFunc>) {
        if let Ok(mut route) = ROUTER.write() {
            route.auth_func = auth_func;
        }
    }

    pub fn use_router(another: Route) -> Result<(), String> {
        if let Ok(mut route) = ROUTER.write() {
            route.replace_with(another);
            return Ok(());
        }

        Err(String::from(
            "Unable to define the router, please try again...",
        ))
    }

    pub(crate) fn add_route(method: REST, uri: RequestPath, callback: RouteHandler) {
        if let Ok(mut route) = ROUTER.write() {
            route.add(method, uri, callback);
        }
    }

    pub(crate) fn add_static(method: REST, uri: Option<RequestPath>, path: PathBuf) {
        if let Ok(mut route) = ROUTER.write() {
            match uri {
                Some(u) => route.add(method, u, RouteHandler(None, Some(path))),
                None => route.set_static(method, path),
            }
        }
    }

    fn add(&mut self, method: REST, uri: RequestPath, callback: RouteHandler) {
        if let Some(r) = self.store.get_mut(&method) {
            //find, insert, done.
            r.insert(uri, callback);
            return;
        }

        let mut map = RouteMap::new();
        map.insert(uri, callback);
        self.store.insert(method, map);
    }

    fn set_static(&mut self, method: REST, path: PathBuf) {
        if !path.exists() || !path.is_dir() {
            panic!("The static path must point to a folder");
        }

        let static_route = StaticLocRoute {
            location: path,
            black_list: HashSet::new(),
            white_list: HashSet::new(),
        };

        if let Some(r) = self.store.get_mut(&method) {
            //find, insert, done.
            r.static_path.replace(static_route);
            return;
        }

        let mut map = RouteMap::new();
        map.static_path.replace(static_route);
        self.store.insert(method, map);
    }

    fn replace_with(&mut self, mut another: Route) {
        self.store = another.store;
        self.auth_func = another.auth_func.take();
    }
}

pub trait Router {
    fn get(&mut self, uri: RequestPath, callback: Callback) -> &mut Self;
    fn patch(&mut self, uri: RequestPath, callback: Callback) -> &mut Self;
    fn post(&mut self, uri: RequestPath, callback: Callback) -> &mut Self;
    fn put(&mut self, uri: RequestPath, callback: Callback) -> &mut Self;
    fn delete(&mut self, uri: RequestPath, callback: Callback) -> &mut Self;
    fn options(&mut self, uri: RequestPath, callback: Callback) -> &mut Self;
    fn other(&mut self, method: &str, uri: RequestPath, callback: Callback) -> &mut Self;
    fn all(&mut self, uri: RequestPath, callback: Callback) -> &mut Self;
    fn use_static(&mut self, path: PathBuf) -> &mut Self;
    fn use_custom_static(&mut self, uri: RequestPath, path: PathBuf) -> &mut Self;
}

impl Router for Route {
    fn get(&mut self, uri: RequestPath, callback: Callback) -> &mut Self {
        self.add(REST::GET, uri, RouteHandler(Some(callback), None));
        self
    }

    fn patch(&mut self, uri: RequestPath, callback: Callback) -> &mut Self {
        self.add(REST::PATCH, uri, RouteHandler(Some(callback), None));
        self
    }

    fn post(&mut self, uri: RequestPath, callback: Callback) -> &mut Self {
        self.add(REST::POST, uri, RouteHandler(Some(callback), None));
        self
    }

    fn put(&mut self, uri: RequestPath, callback: Callback) -> &mut Self {
        self.add(REST::PUT, uri, RouteHandler(Some(callback), None));
        self
    }

    fn delete(&mut self, uri: RequestPath, callback: Callback) -> &mut Self {
        self.add(REST::DELETE, uri, RouteHandler(Some(callback), None));
        self
    }

    fn options(&mut self, uri: RequestPath, callback: Callback) -> &mut Self {
        self.add(REST::OPTIONS, uri, RouteHandler(Some(callback), None));
        self
    }

    fn other(&mut self, method: &str, uri: RequestPath, callback: Callback) -> &mut Self {
        if method.is_empty() {
            panic!("Must provide a valid method!");
        }

        let request_method = REST::OTHER(method.to_uppercase());
        self.add(request_method, uri, RouteHandler(Some(callback), None));

        self
    }

    /// Function 'all' will match the uri on all request methods. Note that the "match all" paradigm
    /// is used in this framework as a safe fallback, which means that if a different callback
    /// has been defined for the same uri but under a explicitly defined request method (e.g. get,
    /// post, etc.), it will be matched and invoked instead of the "match all" callback functions.
    fn all(&mut self, uri: RequestPath, callback: Callback) -> &mut Self {
        self.other("*", uri, callback)
    }


    /// # Example
    ///
    /// ```
    /// extern crate rusty_express;
    /// use rusty_express::prelude::*;
    /// use std::path::PathBuf;
    /// fn main() {
    ///    // define http server now
    ///    let mut server = HttpServer::new();
    ///    server.set_pool_size(8);
    ///    server.use_static(PathBuf::from(r".\static"));
    /// }
    /// ```
    fn use_static(&mut self, path: PathBuf) -> &mut Self {
        self.set_static(REST::GET, path);
        self
    }

    fn use_custom_static(&mut self, uri: RequestPath, path: PathBuf) -> &mut Self {
        self.add(REST::GET, uri, RouteHandler(None, Some(path)));
        self
    }
}

pub(crate) trait RouteSeeker {
    fn seek(
        method: &REST,
        uri: &str,
        header_only: bool,
        tx: channel::Sender<(RouteHandler, HashMap<String, String>)>,
    );
}

impl RouteSeeker for Route {
    fn seek(
        method: &REST,
        uri: &str,
        header_only: bool,
        tx: channel::Sender<(RouteHandler, HashMap<String, String>)>,
    ) {
        let mut result = RouteHandler(None, None);
        let mut params = HashMap::new();

        if let Ok(route_store) = ROUTER.read() {
            if let Some(routes) = route_store.store.get(method) {
                result = routes.seek_path(uri, &mut params);
            } else if header_only {
                //if a header only request, fallback to search with REST::GET
                if let Some(routes) = route_store.store.get(&REST::GET) {
                    result = routes.seek_path(uri, &mut params);
                }
            }

            if result.is_none() {
                if let Some(all_routes) = route_store.store.get(&ROUTE_FOR_ALL) {
                    result = all_routes.seek_path(uri, &mut params);
                }
            }
        }

        if let Err(e) = tx.send((result, params)) {
            debug::print("Unable to find the route handler", InfoLevel::Error);
        }
    }
}

fn search_wildcard_router(routes: &HashMap<String, RegexRoute>, uri: &str) -> RouteHandler {
    let mut result = RouteHandler(None, None);
    for (_, route) in routes.iter() {
        if route.regex.is_match(&uri) {
            result = route.handler.clone();
            break;
        }
    }

    result
}

fn search_params_router(
    route_head: &RouteTrie,
    uri: &str,
    params: &mut HashMap<String, String>
) -> RouteHandler
{
    let raw_segments: Vec<String> = uri
        .trim_matches('/')
        .split('/')
        .map(String::from)
        .collect();

    params.reserve(raw_segments.len());
    let result =
        RouteTrie::find(route_head, raw_segments.as_slice(), params);

    params.shrink_to_fit();
    result
}

fn search_static_router(
    path: &StaticLocRoute,
    raw_uri: &str,
) -> Result<RouteHandler, ()>
{
    // check if static path can be met
    let mut normalized_uri = path.location.clone();
    normalized_uri.push(raw_uri.trim_start_matches(|x| x == '.' || x == '/'));

    let meta = match fs::metadata(&normalized_uri) {
        Ok(m) => m,
        _ => return Ok(RouteHandler::default()),
    };

    // only if the file exists
    if meta.is_file() {
        // the requested file exists, now check white-list and black-list, in this order
        let ext = match normalized_uri.extension() {
            Some(e) => {
                if let Some(e_str) = e.to_str() {
                    ["*.", e_str].join("")
                } else {
                    String::new()
                }
            },
            _ => String::new()
        };

        let file = match normalized_uri.file_name() {
            Some(f) => {
                if let Some(f_str) = f.to_str() {
                    String::from(f_str)
                } else {
                    String::new()
                }
            },
            _ => String::new()
        };

        if !path.white_list.is_empty()
            && (!ext.is_empty() && !path.white_list.contains(&ext))
            && (!file.is_empty() && !path.white_list.contains(&file))
        {
            return Err(());
        }

        if !path.black_list.is_empty()
            && ((!ext.is_empty() && path.black_list.contains(&ext))
            || (!file.is_empty() && path.black_list.contains(&file)))
        {
            return Err(());
        }

        return Ok(RouteHandler(None, Some(normalized_uri)))
    }

    Ok(RouteHandler::default())
}

pub(crate) struct RouteHandler(Option<Callback>, Option<PathBuf>);

impl RouteHandler {
    pub(crate) fn new(cb: Option<Callback>, path: Option<PathBuf>) -> Self {
        RouteHandler(cb, path)
    }

    pub(crate) fn is_some(&self) -> bool {
        self.0.is_some() || self.1.is_some()
    }

    pub(crate) fn is_none(&self) -> bool {
        self.0.is_none() && self.1.is_none()
    }

    pub(crate) fn execute(&mut self, req: &Box<Request>, resp: &mut Box<Response>) {
        assert!(self.is_some());

        if let Some(cb) = self.0.take() {
            cb(req, resp);
            return;
        }

        if let Some(path) = self.1.take() {
            resp.send_file_from_path_async(path);
        }
    }

    fn update_handler(mut handler: RouteHandler, file_name: &str) -> RouteHandler {
        if file_name.is_empty() {
            return handler;
        }

        if let Some(mut p) = handler.1.take() {
            p.push(file_name);
            handler.1.replace(p);
        };

        handler
    }
}

impl Default for RouteHandler {
    fn default() -> Self {
        RouteHandler(None, None)
    }
}

impl Clone for RouteHandler {
    fn clone(&self) -> Self {
        RouteHandler(self.0, self.1.clone())
    }
}

#[cfg(test)]
mod route_test {
    use super::{Field, RouteMap};
    use regex::*;

    #[test]
    fn params_parser_test_one() {
        let regex = Regex::new("a=[/]bdc").unwrap();
        let base = vec![
            Field::new(String::from("root"), false, None),
            Field::new(String::from("api"), false, None),
            Field::new(String::from("Tes中t"), true, Some(regex)),
            Field::new(String::from("this."), false, None),
            Field::new(String::from("check"), true, None),
        ];

        let test = RouteMap::params_parser("/root/api/:Tes中t(a=[/]bdc)/this./:check/");
        assert_eq!(test.len(), base.len());

        for (base_field, test_field) in base.iter().zip(&test) {
            assert_eq!(base_field, test_field);
        }
    }
}
