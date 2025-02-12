use std::sync::mpsc;
use std::sync::Mutex;

use anyhow::Result;
use log::{debug, info, trace, warn};
use websocket::client::sync::Client;
use websocket::stream::sync::TcpStream;
use websocket::url::Url;
use websocket::WebSocketError;
use websocket::{ClientBuilder, OwnedMessage};

use crate::types::{parse_raw_message, Message};

pub struct WebSocketConnection {
    sender: Mutex<websocket::sender::Writer<TcpStream>>,
    process_id: Option<u32>,
}

// TODO websocket::sender::Writer is not :Debug...
impl std::fmt::Debug for WebSocketConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        write!(f, "WebSocketConnection {{}}")
    }
}

impl WebSocketConnection {
    pub fn new(
        ws_url: &Url,
        process_id: Option<u32>,
        messages_tx: mpsc::Sender<Message>,
    ) -> Result<Self> {
        let connection = Self::websocket_connection(ws_url)?;
        let (websocket_receiver, sender) = connection.split()?;

        std::thread::spawn(move || {
            trace!("Starting msg dispatching loop");
            Self::dispatch_incoming_messages(websocket_receiver, messages_tx, process_id);
            trace!("Quit loop msg dispatching loop");
        });

        Ok(Self {
            sender: Mutex::new(sender),
            process_id,
        })
    }

    pub fn shutdown(&self) {
        trace!(
            "Shutting down WebSocket connection for Chrome {:?}",
            self.process_id
        );
        if self.sender.lock().unwrap().shutdown_all().is_err() {
            debug!(
                "Couldn't shut down WS connection for Chrome {:?}",
                self.process_id
            );
        }
    }

    fn dispatch_incoming_messages(
        mut receiver: websocket::receiver::Reader<TcpStream>,
        messages_tx: mpsc::Sender<Message>,
        process_id: Option<u32>,
    ) {
        for ws_message in receiver.incoming_messages() {
            match ws_message {
                Err(error) => match error {
                    WebSocketError::NoDataAvailable => {
                        debug!("WS Error Chrome #{:?}: {}", process_id, error);
                        break;
                    }
                    WebSocketError::IoError(err) => {
                        debug!("WS IO Error for Chrome #{:?}: {}", process_id, err);
                        break;
                    }
                    _ => panic!(
                        "Unhandled WebSocket error for Chrome #{:?}: {:?}",
                        process_id, error
                    ),
                },
                Ok(message) => {
                    if let OwnedMessage::Text(message_string) = message {
                        if let Ok(message) = parse_raw_message(&message_string) {
                            if messages_tx.send(message).is_err() {
                                break;
                            }
                        } else {
                            trace!(
                                "Incoming message isn't recognised as event or method response: {}",
                                message_string
                            );
                        }
                    } else {
                        panic!("Got a weird message: {:?}", message);
                    }
                }
            }
        }

        info!("Sending shutdown message to message handling loop");
        if messages_tx.send(Message::ConnectionShutdown).is_err() {
            warn!("Couldn't send message to transport loop telling it to shut down");
        }
    }

    pub fn websocket_connection(ws_url: &Url) -> Result<Client<TcpStream>> {
        let client = ClientBuilder::from_url(ws_url).connect_insecure()?;

        debug!("Successfully connected to WebSocket: {}", ws_url);

        Ok(client)
    }

    pub fn send_message(&self, message_text: &str) -> Result<()> {
        let message = websocket::Message::text(message_text);
        let mut sender = self.sender.lock().unwrap();
        sender.send_message(&message)?;
        Ok(())
    }
}

impl Drop for WebSocketConnection {
    fn drop(&mut self) {
        info!("dropping websocket connection");
    }
}
