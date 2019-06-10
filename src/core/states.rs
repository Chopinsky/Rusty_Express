#![allow(dead_code)]

use std::thread::JoinHandle;
use std::net::SocketAddr;

use super::config::ServerConfig;
use super::router::Route;
use crate::channel::{self, TryRecvError, SendError};
use crate::support::debug::{self, InfoLevel};
use crate::support::session::*;

pub enum ControlMessage {
    Terminate,
    HotReloadConfig,
    HotLoadRouter(Route),
    HotLoadConfig(ServerConfig),
    Custom(String),
}

pub struct AsyncController(channel::Sender<ControlMessage>, SocketAddr);

impl AsyncController {
    fn new(messenger: channel::Sender<ControlMessage>, addr: SocketAddr) -> Self {
        AsyncController(messenger, addr)
    }

    pub fn send(&self, message: ControlMessage) -> Result<(), SendError<ControlMessage>> {
        self.0.send(message)?;

        match message {
            ControlMessage::Terminate => {
                //TODO: connect to self address --
                // let s = TcpStream::connect(self.1)
                // if let Ok(ss) = s { let _ = ss.shutdown(Shutdown::Both); }
            }
            _ => {}
        };

        Ok(())
    }
}

pub struct ServerStates {
    running: bool,
    courier_channel: (
        channel::Sender<ControlMessage>,
        channel::Receiver<ControlMessage>,
    ),
    socket_addr: SocketAddr,
    session_auto_clean_handler: Option<JoinHandle<()>>,
}

impl ServerStates {
    pub fn new() -> Self {
        ServerStates {
            running: false,
            courier_channel: channel::bounded(1),
            socket_addr: SocketAddr::from(([127, 0, 0, 1], 8080)),
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

    pub(crate) fn set_port(&mut self, port: u16) {
        self.socket_addr.set_port(port);
    }

    #[inline]
    pub(crate) fn get_courier_sender(&self) -> AsyncController {
        AsyncController::new(self.courier_channel.0.clone(), self.socket_addr)
    }

    pub(crate) fn courier_deliver(&self, msg: ControlMessage) -> Result<(), channel::SendError<ControlMessage>> {
        self.courier_channel.0.send(msg)
    }

    #[inline]
    pub(crate) fn courier_fetch(&self) -> Option<ControlMessage> {
        match self.courier_channel.1.try_recv() {
            Ok(msg) => Some(msg),
            Err(TryRecvError::Empty) => None,
            Err(e) => {
                debug::print(
                    &format!("Hot load channel disconnected: {:?}", e),
                    InfoLevel::Warning,
                );
                None
            }
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
