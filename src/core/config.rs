#![allow(unused_variables)]
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use chrono;
use core::states::StatesInteraction;
use support::common::*;

lazy_static! {
    static ref VIEW_ENGINES: Arc<RwLock<HashMap<String, Box<ViewEngine>>>> = Arc::new(RwLock::new(HashMap::new()));
}

pub type ViewEngine = fn(Box<String>, EngineContext) -> Box<String>;

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
            pool_size: 8,
            read_timeout: 512,
            write_timeout: 512,
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

    pub fn use_default_header(&mut self, header: &HashMap<String, String>) {
        self.meta_data.header = header.clone();
    }

    pub fn set_default_header(&mut self, field: String, value: String, replace: bool) {
        self.meta_data.header.add(&field[..], value, replace);
    }

    pub fn set_session_auto_clean(&mut self, auto_clean_period: Duration) {
        self.session_auto_clean_period = std_to_chrono(auto_clean_period);
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

pub struct EngineContext {
    value: String,
    children: HashMap<String, EngineContext>,
}

pub trait DefineViewEngine {
    fn view_engine(extension: &str, engine: ViewEngine);
}

impl DefineViewEngine for ServerConfig {
    fn view_engine(extension: &str, engine: ViewEngine) {
        if extension.is_empty() { return; }

        if let Ok(mut engines) = VIEW_ENGINES.write() {
            engines.insert(extension.to_owned(), Box::new(engine));
        }
    }
}

//pub trait ViewParser {
//    fn parse_view()
//}

pub struct ConnMetadata {
    header: HashMap<String, String>,
    default_pages: HashMap<u16, String>,
    state_interaction: StatesInteraction,
}

impl ConnMetadata {
    pub fn new() -> Self {
        ConnMetadata {
            header: HashMap::new(),
            default_pages: HashMap::new(),
            state_interaction: StatesInteraction::None,
        }
    }

    #[inline]
    pub fn get_default_header(&self) -> HashMap<String, String> {
        self.header.to_owned()
    }

    #[inline]
    pub fn get_default_pages(&self) -> &HashMap<u16, String> {
        &self.default_pages
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
            default_pages: self.default_pages.clone(),
            state_interaction: self.state_interaction.clone(),
        }
    }
}