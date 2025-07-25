use serde_derive::Deserialize;
use tcp::TcpServerConfig;

use crate::server_backends::websocket::WebsocketServerConfig;

pub mod http;
pub mod tcp;
pub mod websocket;

#[derive(Clone, Debug, Deserialize)]
pub struct ServersConfig {
    pub tcp: Option<TcpServerConfig>,
    pub websocket: Option<WebsocketServerConfig>,
}
