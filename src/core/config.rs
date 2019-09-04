#![allow(dead_code)]

use std::cmp;
use std::fs::File;
use std::io::Read;
use std::ops::Div;
use std::sync::Arc;
use std::time::Duration;

use crate::hashbrown::HashMap;
use crate::num_cpus;
use crate::parking_lot::RwLock;
use crate::support::common::*;
use native_tls::{Identity, TlsAcceptor};
use std::mem::MaybeUninit;

//TODO: load config from file, e.g. config.toml?

/*
lazy_static! {
    static ref VIEW_ENGINES: RwLock<HashMap<String, Box<ViewEngine>>> = RwLock::new(HashMap::new());
    static ref METADATA_STORE: RwLock<ConnMetadata> = RwLock::new(ConnMetadata::new());
}
*/

static mut VIEW_ENGINES: MaybeUninit<RwLock<HashMap<String, Box<ViewEngine>>>> = MaybeUninit::uninit();
static mut METADATA_STORE: MaybeUninit<RwLock<ConnMetadata>> = MaybeUninit::uninit();

pub struct ServerConfig {
    pool_size: usize,
    read_timeout: u16,
    write_timeout: u16,
    read_limit: usize,
    tls_path: &'static str,
    use_session_autoclean: bool,
    session_auto_clean_period: Option<Duration>,
}

impl ServerConfig {
    #[inline]
    pub fn new() -> Self {
        Default::default()
    }

    #[inline]
    pub fn get_pool_size(&self) -> usize {
        self.pool_size
    }

    #[inline]
    pub fn set_pool_size(&mut self, size: usize) {
        self.pool_size = size;
    }

    #[inline]
    pub fn get_read_timeout(&self) -> u16 {
        self.read_timeout
    }

    #[inline]
    pub fn set_read_timeout(&mut self, timeout: u16) {
        self.read_timeout = timeout;
    }

    #[inline]
    pub fn get_write_timeout(&self) -> u16 {
        self.write_timeout
    }

    #[inline]
    pub fn set_write_timeout(&mut self, timeout: u16) {
        self.write_timeout = timeout;
    }

    /// The size of each request in bytes. If a request arrives with a larger size, we will drop the
    /// request with an "Access Denied" message. If setting to 0, we will not enforce the size limit
    /// check and we will keep reading the request until read-timeout, which is default to 512ms,
    /// but can be changed with teh `set_read_timeout` function.
    #[inline]
    pub fn set_read_limit(&mut self, limit: usize) {
        self.read_limit = limit;
    }

    /// Get the read limit size in bytes.
    #[inline]
    pub fn get_read_limit(&self) -> usize {
        self.read_limit
    }

    #[inline]
    pub fn set_session_auto_clean(&mut self, auto_clean: bool) {
        self.use_session_autoclean = auto_clean;
    }

    #[inline]
    pub fn get_session_auto_clean(&self) -> bool {
        self.use_session_autoclean
    }

    #[inline]
    pub fn set_tls_path(&mut self, path: &'static str) {
        self.tls_path = path;
    }

    #[inline]
    pub fn tls_path(&self) -> &str {
        self.tls_path
    }

    #[inline]
    pub(crate) fn build_tls_acceptor(&mut self) -> Option<Arc<TlsAcceptor>> {
        if self.tls_path.is_empty() {
            return None;
        }

        // read the identity from the file
        let mut file = File::open(self.tls_path).unwrap();
        let mut content = vec![];
        file.read_to_end(&mut content).unwrap();

        // create the acceptor using the provided identity
        let identity = Identity::from_pkcs12(&content, "hunter2").unwrap();
        let acceptor = Arc::new(TlsAcceptor::new(identity).unwrap());
        self.tls_path = "";

        Some(acceptor)
    }

    #[inline]
    pub fn clear_session_auto_clean(&mut self) {
        self.session_auto_clean_period = None;
    }

    pub fn get_session_auto_clean_period(&self) -> Option<Duration> {
        self.session_auto_clean_period
    }

    #[inline]
    pub fn set_session_auto_clean_period(&mut self, auto_clean_sec: Duration) {
        self.session_auto_clean_period = Some(auto_clean_sec);
    }

    pub fn use_default_header(header: HashMap<String, String>) {
        let mut store = Self::metadata().write();
        (*store).header = header;
    }

    pub fn set_default_header(field: String, value: String, replace: bool) {
        let mut store = Self::metadata().write();
        (*store).header.add(&field[..], value, replace, false);
    }

    pub fn set_status_page_generator(status: u16, generator: PageGenerator) {
        if status > 0 {
            let mut store = Self::metadata().write();
            (*store).status_page_generators.insert(status, generator);
        }
    }

    pub(crate) fn load_server_params(&self) -> (u64, u64, usize) {
        (
            u64::from(self.get_read_timeout()),
            u64::from(self.get_write_timeout()),
            self.get_read_limit().div(512),
        )
    }

    #[inline]
    fn metadata<'a>() -> &'a mut RwLock<ConnMetadata> {
        unsafe { &mut *METADATA_STORE.as_mut_ptr() }
    }

    fn view_engines<'a>() -> &'a mut RwLock<HashMap<String, Box<ViewEngine>>> {
        unsafe { &mut *VIEW_ENGINES.as_mut_ptr() }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        unsafe {
            VIEW_ENGINES.as_mut_ptr().write(RwLock::new(HashMap::new()));
            METADATA_STORE.as_mut_ptr().write(RwLock::new(ConnMetadata::new()));
        }

        let path = option_env!("TLS_PATH").unwrap_or("");

        ServerConfig {
            pool_size: cmp::max(4 * num_cpus::get(), 8),
            read_timeout: 512,
            write_timeout: 0,
            read_limit: 0,
            tls_path: path,
            use_session_autoclean: false,
            session_auto_clean_period: Some(Duration::from_secs(3600)),
        }
    }
}

/// Function type alias `ViewEngine` represents the function signature required for the external
/// view engine framework to be used in the Rusty_Express. Each engine shall be specific to handle one
/// type of html-template. The 1st parameter represents the raw template content in string format,
/// while the 2nd parameter represents the rendering context -- the information required to render
/// the template into customisable webpage.
pub type ViewEngine = fn(&mut String, Box<dyn EngineContext + Send + Sync>) -> u16;

/// In order to streamline the way to supply rendering context information to the underlying template
/// engines, the `EngineContext` trait is required to be implemented by the `ViewEngine` framework's
/// data model objects. However, the framework can choose other means to obtain context info, under
/// which case, the `display` function can be no-op (i.e. always return empty string wrapped in Ok()
/// as the return value)
///
/// # Examples
/// ```no_run
/// use rusty_express::prelude::*;
///
/// pub struct RenderModel {
///     id: String,
///     user: String,
///     email: String,
/// }
///
/// impl EngineContext for RenderModel {
///     fn display(&self, field: &str) -> Result<String, String> {
///         match field {
///             "User" => Ok(self.user.to_owned()),
///             "ID" => Ok(self.id.to_owned()),
///             "Email" => Ok(self.email.to_owned()),
///             _ => Err(&format_err!("Unable to provide information for the key: {}", field)[..]),
///         }
///     }
/// }
/// ```
pub trait EngineContext {
    /// `display` function should be implemented to provide the rendering context information for the
    /// template engine. For example, if the template engine encounters something like `<p>{{msg}}</p>`,
    /// and the `context.display("msg")` returns `Ok(String::from("A Secret Message!"))`, then the
    /// rendered content could be `<p>A Secret Message!</p>`
    fn display(&self, field: &str) -> Result<String, String>;
}

pub trait ViewEngineDefinition {
    fn view_engine(extension: &str, engine: ViewEngine);
}

impl ViewEngineDefinition for ServerConfig {
    fn view_engine(extension: &str, engine: ViewEngine) {
        if extension.is_empty() {
            return;
        }

        let mut engines = ServerConfig::view_engines().write();;
        (*engines).insert(extension.to_owned(), Box::new(engine));
    }
}

pub trait ViewEngineParser {
    fn template_parser<T: EngineContext + Send + Sync + 'static>(
        extension: &str,
        source: Vec<u8>,
        context: Box<T>,
    ) -> (u16, Vec<u8>);
}

impl ViewEngineParser for ServerConfig {
    fn template_parser<T: EngineContext + Send + Sync + 'static>(
        extension: &str,
        source: Vec<u8>,
        context: Box<T>,
    ) -> (u16, Vec<u8>) {
        if extension.is_empty() {
            return (0, Vec::new());
        }

        match String::from_utf8(source) {
            Ok(mut s) => {
                if let Some(engine) = ServerConfig::view_engines().read().get(extension) {
                    let code = engine(&mut s, context);
                    return (code, Vec::from(s.as_bytes()));
                }

                (404, Vec::new())
            }
            Err(err) => (404, Vec::new()),
        }
    }
}

pub type PageGenerator = fn() -> String;

pub struct ConnMetadata {
    header: HashMap<String, String>,
    status_page_generators: HashMap<u16, PageGenerator>,
}

impl ConnMetadata {
    pub fn new() -> Self {
        ConnMetadata {
            header: HashMap::new(),
            status_page_generators: HashMap::new(),
        }
    }

    #[inline]
    pub fn get_default_header() -> Option<HashMap<String, String>> {
        let store = ServerConfig::metadata().read();
        if !store.header.is_empty() {
            return Some(store.header.clone());
        }

        None
    }

    #[inline]
    pub(crate) fn get_status_pages(status: u16) -> Option<PageGenerator> {
        let store = ServerConfig::metadata().read();
        if store.status_page_generators.is_empty() {
            return None;
        }

        store.status_page_generators.get(&status).cloned()
    }
}
