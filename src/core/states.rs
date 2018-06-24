#![allow(unused_variables)]

use std::thread::*;
use super::http::{Request, Response};
use support::session::*;

pub struct ServerStates {
    going_to_shutdown: bool,
    session_auto_clean_handler: Option<JoinHandle<()>>,
}

impl ServerStates {
    pub fn new() -> Self {
        ServerStates {
            going_to_shutdown: false,
            session_auto_clean_handler: None
        }
    }

    pub fn is_terminating(&self) -> bool {
        self.going_to_shutdown
    }

    pub fn ack_to_terminate(&mut self) {
        self.going_to_shutdown = true;
    }

    pub fn set_session_handler(&mut self, handler: Option<JoinHandle<()>>) {
        self.session_auto_clean_handler = handler;
    }

    pub fn drop_session_auto_clean(&mut self) {
        if let Some(handler) = self.session_auto_clean_handler.take() {
            ExchangeConfig::auto_clean_stop();
        }
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
#[deprecated(since = "0.3.0", note = "This feature will be removed in 0.3.3")]
pub enum StatesInteraction {
    WithRequest,
    WithResponse,
    Both,
    None,
}

#[deprecated(since = "0.3.0", note = "This feature will be removed in 0.3.3")]
pub type RequireStateUpdates = bool;

#[deprecated(since = "0.3.0", note = "This feature will be removed in 0.3.3")]
pub trait StatesProvider {
    fn interaction_stage(&self) -> StatesInteraction;
    fn on_request(&self, req: &mut Box<Request>) -> RequireStateUpdates;
    fn on_response(&self, resp: &mut Box<Response>) -> RequireStateUpdates;
    fn update(&mut self, req: &Box<Request>, resp: Option<&Box<Response>>);
}

#[deprecated(since = "0.3.0", note = "This feature will be removed in 0.3.3")]
pub struct EmptyState {}

#[deprecated(since = "0.3.0", note = "This feature will be removed in 0.3.3")]
impl Clone for EmptyState {
    fn clone(&self) -> Self { EmptyState {} }
}

#[deprecated(since = "0.3.0", note = "This feature will be removed in 0.3.3")]
impl StatesProvider for EmptyState {
    #[inline]
    fn interaction_stage(&self) -> StatesInteraction {
        StatesInteraction::None
    }

    #[inline]
    fn on_request(&self, req: &mut Box<Request>) -> RequireStateUpdates {
        false
    }

    #[inline]
    fn on_response(&self, resp: &mut Box<Response>) -> RequireStateUpdates {
        false
    }

    #[inline]
    fn update(&mut self, req: &Box<Request>, resp: Option<&Box<Response>>) { }
}