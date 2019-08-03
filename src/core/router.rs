#![allow(dead_code)]
#![allow(clippy::borrowed_box)]

use std::fmt;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use crate::channel;
use crate::core::http::{Request, Response, ResponseWriter};
use crate::core::syncstore::StaticStore;
use crate::hashbrown::{HashMap, HashSet};
use crate::regex::Regex;
use crate::support::common::cpu_relax;
use crate::support::{
    common::MapUpdates,
    debug::{self, InfoLevel},
    Field, RouteTrie,
};

//TODO: impl route caching: 1) only explicit and wildcard will get cached ... especially the wildcard
//      one. 2) store uri in the "method:path" format.

static mut ROUTER: StaticStore<(Route, AtomicUsize)> = StaticStore::init();
static mut ROUTE_CACHE: StaticStore<HashMap<(REST, String), RouteHandler>> = StaticStore::init();

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

impl Default for REST {
    fn default() -> REST {
        REST::GET
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
pub type AuthFunc = fn(&Box<Request>, &str) -> bool;

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

impl Clone for StaticLocRoute {
    fn clone(&self) -> Self {
        StaticLocRoute {
            location: self.location.clone(),
            black_list: self.black_list.clone(),
            white_list: self.white_list.clone(),
        }
    }
}

pub(crate) struct RouteMap {
    explicit: HashMap<String, RouteHandler>,
    explicit_with_params: RouteTrie,
    wildcard: HashMap<String, RegexRoute>,
    static_path: Option<StaticLocRoute>,
    case_sensitive: bool,
}

impl RouteMap {
    pub fn new() -> Self {
        RouteMap {
            explicit: HashMap::new(),
            explicit_with_params: RouteTrie::initialize(),
            wildcard: HashMap::new(),
            static_path: None,
            case_sensitive: false,
        }
    }

    pub fn insert(&mut self, uri: RequestPath, callback: RouteHandler) {
        match uri {
            RequestPath::Explicit(req_uri) => {
                if req_uri.is_empty() || !req_uri.starts_with('/') {
                    panic!("Request path must have valid contents and start with '/'.");
                }

                self.explicit
                    .add(req_uri, callback, false, self.case_sensitive);
            }
            RequestPath::WildCard(req_uri) => {
                if req_uri.is_empty() {
                    panic!("Request path must have valid contents.");
                }

                if self.wildcard.contains_key(req_uri) {
                    return;
                }

                if let Ok(re) = Regex::new(req_uri) {
                    self.wildcard.add(
                        req_uri,
                        RegexRoute::new(re, callback),
                        false,
                        self.case_sensitive,
                    );
                }
            }
            RequestPath::ExplicitWithParams(req_uri) => {
                if !req_uri.contains("/:") && !req_uri.contains(":\\") {
                    self.explicit
                        .add(req_uri, callback, false, self.case_sensitive);
                    return;
                }

                self.explicit_with_params.add(
                    RouteMap::params_parser(req_uri, self.case_sensitive),
                    callback.0,
                    callback.1,
                );
            }
        }
    }

    pub fn case_sensitive(&mut self, allow_case: bool) {
        self.case_sensitive = allow_case;
    }

    pub fn is_case_sensitive(&self) -> bool {
        self.case_sensitive
    }

    fn params_parser(source_uri: &'static str, allow_case: bool) -> Vec<Field> {
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

                let field = if !allow_case {
                    name.to_lowercase()
                } else {
                    name.to_owned()
                };

                Some(Field::new(field, is_param, validation.take()))
            })
            .collect();

        result.reverse();
        result
    }

    fn seek_path(&self, raw_uri: &str, params: &mut HashMap<String, String>) -> RouteHandler {
        if raw_uri.is_empty() {
            return RouteHandler::default();
        }

        let route = self.search(raw_uri, "", "", params);
        if route.is_some() {
            return route;
        }

        /*
         * Exact uri match failed, now try parsing the file name if it contains one.
         */
        if !params.is_empty() {
            params.clear();
        }

        let mut actual_uri = String::with_capacity(raw_uri.len());
        let mut file_name = "";

        for part in raw_uri.split('/') {
            if part.is_empty() || part == ".." || part == "." {
                continue;
            }

            if !part.starts_with('.') && !part.ends_with('.') && part.contains('.') {
                file_name = part;
                continue;
            }

            file_name = "";

            actual_uri.push('/');
            actual_uri.push_str(part);
        }

        // uri doesn't contain a file name, actual_uri === raw_uri, done (and route not found).
        if file_name.is_empty() {
            return RouteHandler::default();
        }

        if actual_uri.is_empty() {
            actual_uri.push_str(raw_uri);
        }

        self.search(&actual_uri, raw_uri, file_name, params)
    }

    fn search(
        &self,
        uri: &str,
        raw_uri: &str,
        file_name: &str,
        params: &mut HashMap<String, String>,
    ) -> RouteHandler {
        let for_file = !file_name.is_empty();

        if let Some(callback) = self.explicit.get(uri) {
            // only exact match can return: callback and no file name, or path with file name (custom
            if (!for_file && callback.0.is_some()) || (for_file && callback.1.is_some()) {
                return RouteHandler::update_handler(callback.clone(), file_name);
            }
        }

        if for_file {
            if let Some(static_path) = self.static_path.as_ref() {
                match search_static_router(&static_path, raw_uri) {
                    Ok(res) => {
                        // if res is some, we're done. Searching with the full sym-link, so we use
                        // the raw uri instead of the /custom/path/to/uri, and we don't want to
                        // append the file name again since we check the file-existence already
                        if res.is_some() {
                            return res;
                        }
                    }
                    Err(_) => {
                        // either not in white-list, or in black-list, quit
                        return RouteHandler::default();
                    }
                }
            }
        }

        if !self.explicit_with_params.is_empty() {
            let result = search_params_router(&self.explicit_with_params, uri, params);

            if (!for_file && result.0.is_some()) || (for_file && result.1.is_some()) {
                return RouteHandler::update_handler(result, file_name);
            }
        }

        if !self.wildcard.is_empty() {
            let result = search_wildcard_router(&self.wildcard, uri);

            if (!for_file && result.0.is_some()) || (for_file && result.1.is_some()) {
                return RouteHandler::update_handler(result, file_name);
            }
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
            static_path: self.static_path.clone(),
            case_sensitive: self.case_sensitive,
        }
    }
}

#[derive(Default)]
pub struct Route {
    store: HashMap<REST, RouteMap>,
    auth_func: Option<AuthFunc>,
}

impl Route {
    pub(crate) fn init() {
        unsafe {
            ROUTER.set((Route::new(), AtomicUsize::new(1)));
            ROUTE_CACHE.set(HashMap::new());
        }
    }

    pub fn new() -> Self {
        Default::default()
    }

    pub fn get_auth_func() -> Option<AuthFunc> {
        Route::read().with(|r| r.auth_func)
    }

    pub fn set_auth_func(auth_func: Option<AuthFunc>) {
        Route::write().with(|r| r.auth_func = auth_func);
    }

    pub fn authorize(request: &Box<Request>, uri: &str) -> bool {
        Route::read().with(|r| match r.auth_func {
            Some(auth_fn) => auth_fn(request, uri),
            None => true,
        })
    }

    pub fn use_router(another: Route) {
        Route::write().with(|r| r.replace_with(another));
    }

    pub fn use_router_async(another: Route) {
        thread::spawn(|| {
            Self::use_router(another);
        });
    }

    pub fn all_case_sensitive(allow_case: bool) {
        Route::write().with(|r| {
            for maps in r.store.values_mut() {
                maps.case_sensitive(allow_case);
            }
        });
    }

    pub fn case_sensitive(method: &REST, allow_case: bool) {
        Route::write().with(|r| {
            if let Some(maps) = r.store.get_mut(method) {
                maps.case_sensitive = allow_case;
            }
        });
    }

    pub fn is_case_sensitive(method: &REST) -> bool {
        Route::read().with(|r| {
            r.store
                .get(method)
                .filter(|maps| maps.case_sensitive)
                .is_some()
        })
    }

    pub(crate) fn add_route(method: REST, uri: RequestPath, callback: RouteHandler) {
        Route::write().with(|r| r.add(method, uri, callback));
    }

    pub(crate) fn add_static(method: REST, uri: Option<RequestPath>, path: PathBuf) {
        Route::write().with(|r| match uri {
            Some(u) => r.add(method, u, RouteHandler(None, Some(path))),
            None => r.set_static(method, path),
        });
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

    //TODO: expose black list and white list setter/getter

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

    fn read() -> RouteGuard<'static> {
        RouteGuard::checkout(true)
    }

    fn write() -> RouteGuard<'static> {
        RouteGuard::checkout(false)
    }
}

pub trait Router {
    fn get(&mut self, uri: RequestPath, callback: Callback) -> &mut dyn Router;
    fn patch(&mut self, uri: RequestPath, callback: Callback) -> &mut dyn Router;
    fn post(&mut self, uri: RequestPath, callback: Callback) -> &mut dyn Router;
    fn put(&mut self, uri: RequestPath, callback: Callback) -> &mut dyn Router;
    fn delete(&mut self, uri: RequestPath, callback: Callback) -> &mut dyn Router;
    fn options(&mut self, uri: RequestPath, callback: Callback) -> &mut dyn Router;
    fn other(&mut self, method: &str, uri: RequestPath, callback: Callback) -> &mut dyn Router;
    fn all(&mut self, uri: RequestPath, callback: Callback) -> &mut dyn Router;
    fn use_static(&mut self, path: PathBuf) -> &mut dyn Router;
    fn use_custom_static(&mut self, uri: RequestPath, path: PathBuf) -> &mut dyn Router;
    fn case_sensitive(&mut self, allow_case: bool, method: Option<REST>);
}

impl Router for Route {
    fn get(&mut self, uri: RequestPath, callback: Callback) -> &mut dyn Router {
        self.add(REST::GET, uri, RouteHandler(Some(callback), None));
        self
    }

    fn patch(&mut self, uri: RequestPath, callback: Callback) -> &mut dyn Router {
        self.add(REST::PATCH, uri, RouteHandler(Some(callback), None));
        self
    }

    fn post(&mut self, uri: RequestPath, callback: Callback) -> &mut dyn Router {
        self.add(REST::POST, uri, RouteHandler(Some(callback), None));
        self
    }

    fn put(&mut self, uri: RequestPath, callback: Callback) -> &mut dyn Router {
        self.add(REST::PUT, uri, RouteHandler(Some(callback), None));
        self
    }

    fn delete(&mut self, uri: RequestPath, callback: Callback) -> &mut dyn Router {
        self.add(REST::DELETE, uri, RouteHandler(Some(callback), None));
        self
    }

    fn options(&mut self, uri: RequestPath, callback: Callback) -> &mut dyn Router {
        self.add(REST::OPTIONS, uri, RouteHandler(Some(callback), None));
        self
    }

    fn other(&mut self, method: &str, uri: RequestPath, callback: Callback) -> &mut dyn Router {
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
    fn all(&mut self, uri: RequestPath, callback: Callback) -> &mut dyn Router {
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
    fn use_static(&mut self, path: PathBuf) -> &mut dyn Router {
        self.set_static(REST::GET, path);
        self
    }

    fn use_custom_static(&mut self, uri: RequestPath, path: PathBuf) -> &mut dyn Router {
        self.add(REST::GET, uri, RouteHandler(None, Some(path)));
        self
    }

    /// Note: this API only affect routes moving forward, and it will not be applied to routes
    /// already in the `Router`.
    fn case_sensitive(&mut self, allow_case: bool, method: Option<REST>) {
        if method.is_none() {
            for maps in self.store.values_mut() {
                maps.case_sensitive = allow_case;
            }

            return;
        }

        if let Some(m) = method {
            if let Some(maps) = self.store.get_mut(&m) {
                maps.case_sensitive = allow_case
            }
        }
    }
}

pub(crate) trait RouteSeeker {
    fn seek(method: &REST, uri: &str, tx: channel::Sender<(RouteHandler, HashMap<String, String>)>);
    fn seek_sync(method: &REST, uri: &str) -> (RouteHandler, HashMap<String, String>);
}

impl RouteSeeker for Route {
    fn seek(
        method: &REST,
        uri: &str,
        tx: channel::Sender<(RouteHandler, HashMap<String, String>)>,
    ) {
        if let Err(e) = tx.send(Self::seek_sync(method, uri)) {
            debug::print("Unable to find the route handler", InfoLevel::Error);
        }
    }

    fn seek_sync(method: &REST, uri: &str) -> (RouteHandler, HashMap<String, String>) {
        //TODO: check cache first

        // keep the route_store in limited scope so we can release the read lock ASAP
        Route::read().with(|r| {
            let mut result = RouteHandler(None, None);
            let mut params = HashMap::new();

            // get from the method
            if let Some(routes) = r.store.get(method) {
                result = routes.seek_path(uri, &mut params);
            }

            // if a header only request, fallback to search with REST::GET
            if result.is_none() && method == &REST::OTHER(String::from("HEADER")) {
                if let Some(routes) = r.store.get(&REST::GET) {
                    result = routes.seek_path(uri, &mut params);
                }
            }

            // otherwise, try the all-match routes
            if result.is_none() {
                if let Some(all_routes) = r.store.get(&REST::OTHER(String::from("*"))) {
                    result = all_routes.seek_path(uri, &mut params);
                }
            }

            (result, params)
        })

        //TODO: Caching the request, also maintain the hash-map if it gets too large
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
    head: &RouteTrie,
    uri: &str,
    params: &mut HashMap<String, String>,
) -> RouteHandler {
    let raw_segments: Vec<String> = uri.trim_matches('/').split('/').map(String::from).collect();

    params.reserve(raw_segments.len());
    let result = RouteTrie::find(head, raw_segments.as_slice(), params);

    params.shrink_to_fit();
    result
}

fn search_static_router(path: &StaticLocRoute, raw_uri: &str) -> Result<RouteHandler, ()> {
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
            }
            _ => String::new(),
        };

        let file = match normalized_uri.file_name() {
            Some(f) => {
                if let Some(f_str) = f.to_str() {
                    String::from(f_str)
                } else {
                    String::new()
                }
            }
            _ => String::new(),
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

        return Ok(RouteHandler(None, Some(normalized_uri)));
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
            assert!(
                handler.0.is_none(),
                "Router error: callback and static files are found for this route ..."
            );

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

/// The router guard struct, holding: 1) (mutable) reference to the underlying route; 2) The reader
/// counter reference; 3) if guarding a read access, or not.
#[doc(hidden)]
struct RouteGuard<'a>(&'a mut Route, &'a AtomicUsize, bool);

impl<'a> RouteGuard<'a> {
    fn checkout(is_reader: bool) -> Self {
        // prepare the base reference
        let r = unsafe { ROUTER.as_mut().unwrap() };

        if is_reader {
            // initial guess, doesn't really matter if we have to compete for the lock
            let mut curr = r.1.load(Ordering::Relaxed);

            // waiting for the write lock to release
            while let Err(old) =
                r.1.compare_exchange(curr, curr + 1, Ordering::Acquire, Ordering::Relaxed)
            {
                let count = if old < 1 {
                    // a writer holds the lock, wait till it releases it
                    curr = 1;
                    16
                } else {
                    // otherwise we're in readers mode, try to grab the lock before it goes away to
                    // a contentious writer.
                    curr = old;
                    4
                };

                // take a break: if a writer has the lock, wait longer to check again.
                cpu_relax(count);
            }
        } else {
            // keep checking until all readers have released and no other writer is holding the lock
            while r
                .1
                .compare_exchange(1, 0, Ordering::SeqCst, Ordering::Relaxed)
                .is_err()
            {
                cpu_relax(8);
            }
        }

        RouteGuard(&mut r.0, &r.1, is_reader)
    }

    fn with<T, F: FnOnce(&mut Route) -> T>(&mut self, f: F) -> T {
        f(self.0)
    }
}

impl<'a> Drop for RouteGuard<'a> {
    fn drop(&mut self) {
        if self.2 {
            // this is a reader guard
            self.1.fetch_sub(1, Ordering::Release);
        } else {
            // this is a writer guard
            self.1.store(1, Ordering::SeqCst);
        }
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
            Field::new(String::from("check"), true, None),
            Field::new(String::from("this."), false, None),
            Field::new(String::from("Tes中t"), true, Some(regex)),
            Field::new(String::from("api"), false, None),
            Field::new(String::from("root"), false, None),
        ];

        let test = RouteMap::params_parser("/root/api/:Tes中t(a=[/]bdc)/this./:check/", true);

        assert_eq!(test.len(), base.len());

        let mut num = 0;
        for (base_field, test_field) in base.iter().zip(&test) {
            assert_eq!(base_field, test_field, "Failed at test case: {}", num);
            num += 1;
        }
    }
}
