use serde_derive::Deserialize;
use tcp::TcpServerConfig;
use udp::UdpServerConfig;

pub mod http;
pub mod tcp;
pub mod udp;
pub mod websocket;

#[derive(Clone, Debug, Deserialize)]
pub struct ServersConfig {
    pub tcp: Option<TcpServerConfig>,
    pub udp: Option<UdpServerConfig>,
}
