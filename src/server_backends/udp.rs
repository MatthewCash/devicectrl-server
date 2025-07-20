use anyhow::{Context, Result, bail};
use devicectrl_common::protocol::udp::{ClientBoundUdpMessage, MAX_MSG_LEN, ServerBoundUdpMessage};
use serde_derive::Deserialize;
use std::{net::SocketAddr, sync::Arc};
use tokio::net::UdpSocket;

use crate::{
    AppState,
    auth::verify_auth,
    devices::{controllers::Controllers, dispatch::process_update_request},
};

#[derive(Clone, Debug, Deserialize)]
pub struct UdpServerConfig {
    listen_on: SocketAddr,
}

async fn handle_conn(
    message: &[u8],
    socket: Arc<UdpSocket>,
    sender: &SocketAddr,
    state: &AppState,
) -> Result<()> {
    let send_message = async |message: &ClientBoundUdpMessage| {
        let data = serde_json::to_vec(message)?;
        if data.len() > MAX_MSG_LEN {
            bail!("prepared udp update message is longer than max!");
        }

        Ok(socket.send_to(&data, sender).await?)
    };

    if let Err(err) = async {
        let message: ServerBoundUdpMessage = serde_json::from_slice(message)?;

        match message {
            ServerBoundUdpMessage::UpdateRequest(auth, update_request) => {
                log::debug!("udp got update request: {:?}", update_request);

                if !verify_auth(&auth, &state.config.auth).await? {
                    send_message(&ClientBoundUdpMessage::NotAuthenticated).await?;
                    return Ok(());
                }

                send_message(&ClientBoundUdpMessage::RequestReceived).await?;

                process_update_request(&update_request, state).await?;
            }
            // FIXME: this should also require authentication
            ServerBoundUdpMessage::StateQuery { device_id } => {
                log::debug!("udp got state query for {}", device_id);

                let devices = state.devices.read().await;
                Controllers::dispatch_query_state(
                    state,
                    devices.get(&device_id).context("failed to find device")?,
                )
                .await?;
            }
            _ => {
                send_message(&ClientBoundUdpMessage::UnknownCommand).await?;
            }
        }

        Result::<()>::Ok(())
    }
    .await
    {
        log::warn!("{:?}", err);
        send_message(&err.into()).await?;
    }

    Ok(())
}

pub async fn start_listening(udp_config: UdpServerConfig, state: Arc<AppState>) -> Result<()> {
    let socket = Arc::new(UdpSocket::bind(&udp_config.listen_on).await?);
    log::info!("udp server listening on {}", &udp_config.listen_on);

    loop {
        let mut buf = [0u8; MAX_MSG_LEN];
        let (size, sender) = socket.recv_from(&mut buf).await?;

        log::debug!("udp server received message from {sender}");

        let socket = socket.clone();
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_conn(&buf[..size], socket, &sender, &state).await {
                log::warn!("{:?}", err.context("Failed to handle udp connection"))
            }
        });
    }
}
