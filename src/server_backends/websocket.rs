use anyhow::{Context, Result};
use devicectrl_common::protocol::tcp::{ClientBoundTcpMessage, ServerBoundTcpMessage};
use futures::{
    SinkExt, TryStreamExt,
    future::{Either, select},
};
use hyper::body::Bytes;
use serde::{Deserialize, de};

use serde_derive::Deserialize;
use std::{net::SocketAddr, pin::pin, sync::Arc};
use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
};
use tokio_rustls::{
    TlsAcceptor,
    rustls::{
        RootCertStore, ServerConfig,
        pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
        server::WebPkiClientVerifier,
    },
    server::TlsStream,
};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

use crate::{
    AppState,
    devices::{controllers::Controllers, dispatch::process_update_request},
    hooks::Hook,
};

#[derive(Clone, Debug, Deserialize)]
pub struct WebsocketServerConfig {
    listen_on: SocketAddr,
    #[serde(rename = "cert_path", deserialize_with = "deserialize_file_path_bytes")]
    cert_bytes: Vec<u8>,
    #[serde(rename = "key_path", deserialize_with = "deserialize_file_path_bytes")]
    key_bytes: Vec<u8>,
    #[serde(
        rename = "client_ca_path",
        deserialize_with = "deserialize_file_path_bytes"
    )]
    client_ca_bytes: Vec<u8>,
}

fn deserialize_file_path_bytes<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: de::Deserializer<'de>,
{
    std::fs::read(String::deserialize(deserializer)?).map_err(de::Error::custom)
}

async fn handle_tcp_conn(
    socket: &mut TcpStream,
    state: &AppState,
    acceptor: TlsAcceptor,
) -> Result<()> {
    let stream = acceptor.accept(socket).await?;

    let mut ws_stream = tokio_tungstenite::accept_async(stream)
        .await
        .context("failed to perform websocket handshake")?;

    let mut notifications = state.hooks.receiver.resubscribe();

    loop {
        match select(ws_stream.try_next(), pin!(notifications.recv())).await {
            Either::Left((message, _)) => {
                match message? {
                    None | Some(Message::Close(_)) => {
                        return Ok(());
                    }
                    Some(Message::Ping(_)) => {
                        ws_stream.send(Message::Pong(Bytes::new())).await?;
                    }
                    Some(Message::Text(text)) => {
                        if let Err(err) = handle_line(&text, &mut ws_stream, state)
                            .await
                            .context("failed to handle websocket message")
                        {
                            log::warn!("{:?}", err);

                            ws_stream
                                .send(Message::Text(
                                    serde_json::to_string::<ClientBoundTcpMessage>(&err.into())?
                                        .into(),
                                ))
                                .await?;
                        }
                    }
                    Some(_) => {
                        log::warn!("Ignoring non-text websocket message");
                    }
                };
            }
            Either::Right((event, _)) => {
                if let Hook::DeviceStateUpdate(notification) = event? {
                    ws_stream
                        .send(Message::Text(
                            serde_json::to_string(&ClientBoundTcpMessage::UpdateNotification(
                                notification,
                            ))?
                            .into(),
                        ))
                        .await?;
                }
            }
        }
    }
}

async fn handle_line(
    text: &str,
    stream: &mut WebSocketStream<TlsStream<&mut TcpStream>>,
    state: &AppState,
) -> Result<()> {
    let message: ServerBoundTcpMessage = serde_json::from_str(text)?;

    let mut send = async |msg: &ClientBoundTcpMessage| {
        Result::<()>::Ok(
            stream
                .send(Message::Text(serde_json::to_string(msg)?.into()))
                .await?,
        )
    };

    match message {
        ServerBoundTcpMessage::UpdateRequest(update_request) => {
            log::debug!("websocket got update request: {:?}", update_request);

            send(&ClientBoundTcpMessage::RequestReceived).await?;

            process_update_request(&update_request, state).await?;
        }
        ServerBoundTcpMessage::StateQuery { device_id } => {
            log::debug!("websocket got state query for {}", device_id);

            let devices = state.devices.read().await;
            Controllers::dispatch_query_state(
                state,
                devices.get(&device_id).context("failed to find device")?,
            )
            .await?;
        }
        _ => {
            send(&ClientBoundTcpMessage::Unimplemented).await?;
        }
    };

    Ok(())
}

pub async fn start_listening(
    websocket_config: WebsocketServerConfig,
    state: Arc<AppState>,
) -> Result<()> {
    let mut root_store = RootCertStore::empty();
    root_store.add(CertificateDer::from_pem_slice(
        &websocket_config.client_ca_bytes,
    )?)?;

    let client_verifier = WebPkiClientVerifier::builder(Arc::new(root_store)).build()?;

    let tls_config = ServerConfig::builder()
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(
            vec![CertificateDer::from_pem_slice(
                &websocket_config.cert_bytes,
            )?],
            PrivateKeyDer::from_pem_slice(&websocket_config.key_bytes)?,
        )?;

    let acceptor = TlsAcceptor::from(Arc::new(tls_config));

    let listener = TcpListener::bind(&websocket_config.listen_on).await?;
    log::info!(
        "websocket server listening on {}",
        &websocket_config.listen_on
    );

    loop {
        let (mut socket, sender) = listener.accept().await?;
        log::debug!("websocket tcp server received connection from {sender}");

        let state = state.clone();
        let acceptor = acceptor.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_tcp_conn(&mut socket, &state, acceptor).await {
                log::warn!(
                    "{:?}",
                    err.context("Failed to handle websocket tcp connection")
                )
            }

            let _ = socket.shutdown().await;
        });
    }
}
