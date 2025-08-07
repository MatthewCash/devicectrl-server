use serde_derive::Deserialize;
use tcp::TcpServerConfig;

use crate::server_backends::{http::HttpServerConfig, websocket::WebsocketServerConfig};

pub mod http;
pub mod tcp;
pub mod websocket;

#[derive(Clone, Debug, Deserialize)]
pub struct ServersConfig {
    pub tcp: Option<TcpServerConfig>,
    pub http: Option<HttpServerConfig>,
    pub websocket: Option<WebsocketServerConfig>,
}
