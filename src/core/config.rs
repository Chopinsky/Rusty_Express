#![allow(unused_variables)]
#![allow(dead_code)]

use std::collections::HashMap;
use std::cmp;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use chrono;
use num_cpus;
use support::common::*;

//TODO: load config from file, e.g. config.toml

lazy_static! {
    static ref VIEW_ENGINES: RwLock<HashMap<String, Box<ViewEngine>>> = RwLock::new(HashMap::new());
}

pub struct ServerConfig {
    pub pool_size: usize,
    pub read_timeout: u16,
    pub write_timeout: u16,
    pub use_session_autoclean: bool,
    session_auto_clean_period: Option<chrono::Duration>,
    meta_data: ConnMetadata,
}

impl ServerConfig {
    pub fn new() -> Self {
        ServerConfig {
            /// Be aware that we will create 4 times more worker threads in separate pools
            /// to support the main pool.
            pool_size: cmp::max(num_cpus::get(), 4),
            read_timeout: 256,
            write_timeout: 1024,
            use_session_autoclean: false,
            session_auto_clean_period: Some(chrono::Duration::seconds(3600)),
            meta_data: ConnMetadata::new(),
        }
    }

    #[inline]
    pub fn get_meta_data(&self) -> ConnMetadata {
        self.meta_data.to_owned()
    }

    pub fn use_default_header(&mut self, header: HashMap<String, String>) {
        self.meta_data.header = Box::new(header);
    }

    pub fn set_default_header(&mut self, field: String, value: String, replace: bool) {
        self.meta_data.header.add(&field[..], value, replace);
    }

    pub fn set_session_auto_clean(&mut self, auto_clean_period: Duration) {
        self.session_auto_clean_period = std_to_chrono(auto_clean_period);
    }

    pub fn set_status_page_generator(&mut self, status: u16, generator: PageGenerator) {
        if status > 0 {
            if let Some(generators) = Arc::get_mut(&mut self.meta_data.status_page_generators) {
                generators.insert(status, generator);
            }
        }
    }

    #[inline]
    pub fn reset_session_auto_clean(&mut self) {
        self.session_auto_clean_period = None;
    }

    pub fn get_session_auto_clean_period(&self) -> Option<Duration> {
        match self.session_auto_clean_period {
            Some(period) => chrono_to_std(period),
            _ => None,
        }
    }
}

/// Function type alias `ViewEngine` represents the function signature required for the external
/// view engine framework to be used in the Rusty_Express. Each engine shall be specific to handle one
/// type of html-template. The 1st parameter represents the raw template content in string format,
/// while the 2nd parameter represents the rendering context -- the information required to render
/// the template into customisable webpage.
pub type ViewEngine = fn(&mut Box<String>, Box<EngineContext + Send + Sync>) -> u16;

/// In order to streamline the way to supply rendering context information to the underlying template
/// engines, the `EngineContext` trait is required to be implemented by the `ViewEngine` framework's
/// data model objects. However, the framework can choose other means to obtain context info, under
/// which case, the `display` function can be no-op (i.e. always return empty string wrapped in Ok()
/// as the return value)
///
/// # Examples
/// ```rust
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
///             _ => Err(&format!("Unable to provide information for the key: {}", field)[..]),
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
        if extension.is_empty() { return; }

        if let Ok(mut engines) = VIEW_ENGINES.write() {
            engines.insert(extension.to_owned(), Box::new(engine));
        }
    }
}

pub trait ViewEngineParser {
    fn template_parser<T: EngineContext + Send + Sync + 'static>(
        extension: &str,
        content:
        &mut Box<String>,
        context: Box<T>) -> u16;
}

impl ViewEngineParser for ServerConfig {
    fn template_parser<T: EngineContext + Send + Sync + 'static>(
        extension: &str,
        content: &mut Box<String>,
        context: Box<T>) -> u16
    {
        if extension.is_empty() { return 0; }

        if let Ok(template_engines) = VIEW_ENGINES.read() {
            if let Some(engine) = template_engines.get(extension) {
                return engine(content, context);
            } else {
                return 404;
            }
        }

        500
    }
}

pub type PageGenerator = fn() -> String;

pub struct ConnMetadata {
    header: Box<HashMap<String, String>>,
    status_page_generators: Arc<Box<HashMap<u16, PageGenerator>>>,
}

impl ConnMetadata {
    pub fn new() -> Self {
        ConnMetadata {
            header: Box::new(HashMap::new()),
            status_page_generators: Arc::new(Box::new(HashMap::new())),
        }
    }

    #[inline]
    pub fn get_default_header(&self) -> HashMap<String, String> {
        (*self.header).clone()
    }

    #[inline]
    pub fn get_status_pages(&self) -> Arc<Box<HashMap<u16, PageGenerator>>> {
        Arc::clone(&self.status_page_generators)
    }
}

impl Clone for ConnMetadata {
    fn clone(&self) -> Self {
        ConnMetadata {
            header: self.header.clone(),
            status_page_generators: self.status_page_generators.clone(),
        }
    }
}