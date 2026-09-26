use anyhow::Result;
use devicectrl_common::{
    SceneId,
    protocol::socket::{ClientBoundSocketMessage, DeviceDescriptor},
};
use serde_derive::Deserialize;
use tokio::task::JoinSet;

use crate::{
    AppState,
    server_backends::{
        http::HttpServerConfig, tcp::TcpServerConfig, websocket::WebsocketServerConfig,
    },
};

pub mod http;
pub mod tcp;
pub mod websocket;

async fn catalog(app_state: &AppState) -> ClientBoundSocketMessage {
    let devices = app_state
        .devices
        .read()
        .await
        .values()
        .map(|device| device.descriptor())
        .collect::<Vec<_>>();
    let scenes = app_state.config.scenes.keys().copied().collect::<Vec<_>>();

    catalog_message(devices, scenes)
}

fn catalog_message(
    mut devices: Vec<DeviceDescriptor>,
    mut scenes: Vec<SceneId>,
) -> ClientBoundSocketMessage {
    devices.sort_unstable_by_key(|device| device.device_id);
    scenes.sort_unstable();

    ClientBoundSocketMessage::Catalog { devices, scenes }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ServersConfig {
    pub tcp: Option<TcpServerConfig>,
    pub http: Option<HttpServerConfig>,
    pub websocket: Option<WebsocketServerConfig>,
}

macro_rules! spawn_servers {
    ( $tasks:expr, $config:expr, $app_state:expr, [$( $field:ident ),*] ) => {
        $(
            if let Some(ref config) = $config.$field {
                $tasks.spawn($field::start_server(
                    config.clone(),
                    $app_state,
                ));
            }
        )*
    };
}

pub async fn start_servers(app_state: &'static AppState) -> Result<()> {
    let config = &app_state.config.servers;

    let mut tasks = JoinSet::new();

    spawn_servers!(tasks, config, app_state, [http, tcp, websocket]);

    while tasks.join_next().await.transpose()?.transpose()?.is_some() {}

    Ok(())
}
