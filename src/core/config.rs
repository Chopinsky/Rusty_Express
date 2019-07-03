#![allow(dead_code)]

use crate::hashbrown::HashMap;
use crate::num_cpus;
use crate::support::common::*;
use parking_lot::RwLock;
use std::cmp;
use std::time::Duration;

//TODO: load config from file, e.g. config.toml?

lazy_static!(
    static ref VIEW_ENGINES: RwLock<HashMap<String, Box<ViewEngine>>> = RwLock::new(HashMap::new());
    static ref METADATA_STORE: RwLock<ConnMetadata> = RwLock::new(ConnMetadata::new());
);

pub struct ServerConfig {
    pool_size: usize,
    read_timeout: u16,
    write_timeout: u16,
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

    #[inline]
    pub fn set_session_auto_clean(&mut self, auto_clean: bool) {
        self.use_session_autoclean = auto_clean;
    }

    #[inline]
    pub fn get_session_auto_clean(&self) -> bool {
        self.use_session_autoclean
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
        let mut store = METADATA_STORE.write();
        (*store).header = header;
    }

    pub fn set_default_header(field: String, value: String, replace: bool) {
        let mut store = METADATA_STORE.write();
        (*store).header.add(&field[..], value, replace, false);
    }

    pub fn set_status_page_generator(status: u16, generator: PageGenerator) {
        if status > 0 {
            let mut store = METADATA_STORE.write();
            (*store).status_page_generators.insert(status, generator);
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            pool_size: cmp::max(4*num_cpus::get(), 8),
            read_timeout: 512,
            write_timeout: 0,
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
pub type ViewEngine = fn(&mut String, Box<EngineContext + Send + Sync>) -> u16;

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

        let mut engines = VIEW_ENGINES.write();
        (*engines).insert(extension.to_owned(), Box::new(engine));
    }
}

pub trait ViewEngineParser {
    fn template_parser<T: EngineContext + Send + Sync + 'static>(
        extension: &str,
        content: &mut String,
        context: Box<T>,
    ) -> u16;
}

impl ViewEngineParser for ServerConfig {
    fn template_parser<T: EngineContext + Send + Sync + 'static>(
        extension: &str,
        content: &mut String,
        context: Box<T>,
    ) -> u16 {
        if extension.is_empty() {
            return 0;
        }

        let template_engines = VIEW_ENGINES.read();
        if let Some(engine) = template_engines.get(extension) {
            return engine(content, context);
        } else {
            return 404;
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
        let store = METADATA_STORE.read();
        if !store.header.is_empty() {
            return Some(store.header.clone());
        }

        None
    }

    #[inline]
    pub(crate) fn get_status_pages(status: u16) -> Option<PageGenerator> {
        let store = METADATA_STORE.read();
        if store.status_page_generators.is_empty() {
            return None;
        }

        store.status_page_generators.get(&status).cloned()
    }
}
