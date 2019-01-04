#![allow(dead_code)]

use super::config::ServerConfig;
use super::router::Route;
use std::thread::*;
use crate::channel::{self, TryRecvError};
use crate::support::session::*;
use crate::support::debug::{self, InfoLevel};

pub enum ControlMessage {
    Terminate,
    HotLoadRouter(Route),
    HotLoadConfig(ServerConfig),
    Custom(String),
}

pub struct ServerStates {
    running: bool,
    courier_channel: (
        channel::Sender<ControlMessage>,
        channel::Receiver<ControlMessage>,
    ),
    session_auto_clean_handler: Option<JoinHandle<()>>,
}

impl ServerStates {
    pub fn new() -> Self {
        ServerStates {
            running: false,
            courier_channel: channel::bounded(1),
            session_auto_clean_handler: None,
        }
    }

    pub fn set_session_handler(&mut self, handler: Option<JoinHandle<()>>) {
        self.session_auto_clean_handler = handler;
    }

    pub fn drop_session_auto_clean(&mut self) {
        if let Some(handler) = self.session_auto_clean_handler.take() {
            ExchangeConfig::auto_clean_stop();
        }
    }

    #[inline]
    pub(crate) fn get_courier_sender(&self) -> channel::Sender<ControlMessage> {
        channel::Sender::clone(&self.courier_channel.0)
    }

    #[inline]
    pub(crate) fn courier_try_recv(&self) -> Option<ControlMessage> {
        match self.courier_channel.1.try_recv() {
            Ok(msg) => Some(msg),
            Err(TryRecvError::Empty) => {
                None
            },
            Err(e) => {
                debug::print(&format!("Hot load channel disconnected: {:?}", e), InfoLevel::Warning);
                None
            },
        }
    }

    #[inline]
    pub(crate) fn toggle_running_state(&mut self, running: bool) {
        self.running = true;
    }

    #[inline]
    pub(crate) fn is_running(&self) -> bool {
        self.running
    }
}
