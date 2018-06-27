#![allow(unused_variables)]
#![allow(dead_code)]

use std::thread::*;
use channel;
use super::config::ServerConfig;
use super::router::Route;
use support::session::*;

pub enum ControlMessage {
    Terminate,
    HotLoadRouter(Route),
    HotLoadConfig(ServerConfig),
    Custom(String),
}

pub struct ServerStates {
    courier_channel: (channel::Sender<ControlMessage>, channel::Receiver<ControlMessage>),
    going_to_shutdown: bool,
    session_auto_clean_handler: Option<JoinHandle<()>>,
}

impl ServerStates {
    pub fn new() -> Self {
        ServerStates {
            courier_channel: channel::bounded(1),
            going_to_shutdown: false,
            session_auto_clean_handler: None,
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

    #[inline]
    pub(crate) fn get_courier_sender(&self) -> channel::Sender<ControlMessage> {
        channel::Sender::clone(&self.courier_channel.0)
    }

    #[inline]
    pub(crate) fn courier_try_recv(&self) -> Option<ControlMessage> {
        self.courier_channel.1.try_recv()
    }
}