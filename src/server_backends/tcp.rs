use anyhow::{Context, Result};
use devicectrl_common::protocol::tcp::{ClientBoundTcpMessage, ServerBoundTcpMessage};
use futures::{
    SinkExt, TryStreamExt,
    future::{Either, select},
};
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
use tokio_util::codec::{Framed, LinesCodec};

use crate::{
    AppState,
    devices::{controllers::Controllers, dispatch::process_update_request},
    hooks::Hook,
};

#[derive(Clone, Debug, Deserialize)]
pub struct TcpServerConfig {
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

async fn handle_conn(
    socket: &mut TcpStream,
    state: &AppState,
    acceptor: TlsAcceptor,
) -> Result<()> {
    let mut stream = Framed::new(acceptor.accept(socket).await?, LinesCodec::new());

    let mut notifications = state.hooks.receiver.resubscribe();

    loop {
        match select(stream.try_next(), pin!(notifications.recv())).await {
            Either::Left((line, _)) => {
                let Some(line) = line? else { return Ok(()) };
                if let Err(err) = handle_line(&line, &mut stream, state)
                    .await
                    .context("failed to handle tcp line")
                {
                    log::warn!("{:?}", err);

                    stream
                        .send(&serde_json::to_string::<ClientBoundTcpMessage>(
                            &err.into(),
                        )?)
                        .await?;
                }
            }
            Either::Right((event, _)) => {
                if let Hook::DeviceStateUpdate(notification) = event? {
                    stream
                        .send(&serde_json::to_string(
                            &ClientBoundTcpMessage::UpdateNotification(notification),
                        )?)
                        .await?;
                }
            }
        }
    }
}

async fn handle_line(
    line: &str,
    stream: &mut Framed<TlsStream<&mut TcpStream>, LinesCodec>,
    state: &AppState,
) -> Result<()> {
    let message: ServerBoundTcpMessage = serde_json::from_str(line)?;

    let mut send = async |msg: &ClientBoundTcpMessage| {
        Result::<()>::Ok(stream.send(&serde_json::to_string(msg)?).await?)
    };

    match message {
        ServerBoundTcpMessage::UpdateRequest(update_request) => {
            log::debug!("tcp got update request: {:?}", update_request);

            send(&ClientBoundTcpMessage::RequestReceived).await?;

            process_update_request(&update_request, state).await?;
        }
        ServerBoundTcpMessage::StateQuery { device_id } => {
            log::debug!("tcp got state query for {}", device_id);

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

pub async fn start_listening(tcp_config: TcpServerConfig, state: Arc<AppState>) -> Result<()> {
    let mut root_store = RootCertStore::empty();
    root_store.add(CertificateDer::from_pem_slice(&tcp_config.client_ca_bytes)?)?;

    let client_verifier = WebPkiClientVerifier::builder(Arc::new(root_store)).build()?;

    let tls_config = ServerConfig::builder()
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(
            vec![CertificateDer::from_pem_slice(&tcp_config.cert_bytes)?],
            PrivateKeyDer::from_pem_slice(&tcp_config.key_bytes)?,
        )?;

    let acceptor = TlsAcceptor::from(Arc::new(tls_config));

    let listener = TcpListener::bind(&tcp_config.listen_on).await?;
    log::info!("tcp server listening on {}", &tcp_config.listen_on);

    loop {
        let (mut socket, sender) = listener.accept().await?;
        log::debug!("tcp server received connection from {sender}");

        let state = state.clone();
        let acceptor = acceptor.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_conn(&mut socket, &state, acceptor).await {
                log::warn!("{:?}", err.context("Failed to handle tcp connection"))
            }

            let _ = socket.shutdown().await;
        });
    }
}
