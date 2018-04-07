#![allow(unused_variables)]
#![allow(dead_code)]
#![allow(deprecated)]

use std::collections::HashMap;
use std::cmp;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use chrono;
use num_cpus;
use super::states::StatesInteraction;
use support::common::*;

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

    #[inline]
    pub fn set_managed_state_interaction(&mut self, interaction: StatesInteraction) {
        self.meta_data.set_state_interaction(interaction);
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

pub type ViewEngine = fn(Box<String>, Box<EngineContext>, &mut Box<String>) -> u16;

pub struct EngineContext {
    value: String,
    children: HashMap<String, Box<EngineContext>>,
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
    fn template_parser(extension: &str, content: Box<String>, context: Box<EngineContext>, output: &mut Box<String>) -> u16;
}

impl ViewEngineParser for ServerConfig {
    fn template_parser(extension: &str, content: Box<String>, context: Box<EngineContext>, output: &mut Box<String>) -> u16 {
        if extension.is_empty() { return 0; }

        if let Ok(template_engines) = VIEW_ENGINES.read() {
            if let Some(engine) = template_engines.get(extension) {
                return engine(content, context, output);
            }
        }

        return 0;
    }
}

pub type PageGenerator = fn() -> String;

pub struct ConnMetadata {
    header: Box<HashMap<String, String>>,
    status_page_generators: Arc<Box<HashMap<u16, PageGenerator>>>,
    state_interaction: StatesInteraction,
}

impl ConnMetadata {
    pub fn new() -> Self {
        ConnMetadata {
            header: Box::new(HashMap::new()),
            status_page_generators: Arc::new(Box::new(HashMap::new())),
            state_interaction: StatesInteraction::None,
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

    #[inline]
    pub fn set_state_interaction(&mut self, interaction: StatesInteraction) {
        self.state_interaction = interaction;
    }

    #[inline]
    pub fn get_state_interaction(&self) -> &StatesInteraction {
        &self.state_interaction
    }
}

impl Clone for ConnMetadata {
    fn clone(&self) -> Self {
        ConnMetadata {
            header: self.header.clone(),
            status_page_generators: self.status_page_generators.clone(),
            state_interaction: self.state_interaction.clone(),
        }
    }
}

//TODO: load config from file, e.g. config.toml