use anyhow::{Context, Result};
use devicectrl_common::protocol::http::{ClientBoundHttpMessage, ServerBoundHttpMessage};
use http_body_util::BodyExt;
use hyper::{Request, Response, StatusCode};
use hyper::{
    body::Incoming,
    server::conn::{http1, http2},
};
use hyper_util::rt::{TokioExecutor, TokioIo};
use serde::{Deserialize, de};
use serde_derive::Deserialize;
use std::{net::SocketAddr, sync::Arc};
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
};

use crate::{AppState, devices::dispatch::process_update_request};

#[derive(Clone, Debug, Deserialize)]
pub struct HttpServerConfig {
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
    state: Arc<AppState>,
    acceptor: TlsAcceptor,
) -> Result<()> {
    let stream = acceptor.accept(socket).await?;
    let protocol = stream.get_ref().1.alpn_protocol().map(|p| p.to_vec());

    let io = TokioIo::new(stream);

    let service = hyper::service::service_fn(|req| handle_request(req, state.clone()));

    match protocol.as_deref() {
        Some(b"h2") => {
            http2::Builder::new(TokioExecutor::new())
                .serve_connection(io, service)
                .await
                .context("failed to serve http/2 connection")?;
        }
        _ => {
            http1::Builder::new()
                .serve_connection(io, service)
                .await
                .context("failed to serve http/1 connection")?;
        }
    }

    Ok(())
}

async fn handle_request(req: Request<Incoming>, state: Arc<AppState>) -> Result<Response<String>> {
    let bytes = req.into_body().collect().await?.to_bytes();
    let message: ServerBoundHttpMessage = serde_json::from_slice(&bytes)?;

    Ok(match message {
        ServerBoundHttpMessage::UpdateRequest(update_request) => {
            log::debug!("http got update request: {:?}", update_request);

            process_update_request(&update_request, &state).await?;

            Response::new(serde_json::to_string(
                &ClientBoundHttpMessage::RequestReceived,
            )?)
        }
        _ => Response::builder()
            .status(StatusCode::NOT_IMPLEMENTED)
            .body(serde_json::to_string(
                &ClientBoundHttpMessage::Unimplemented,
            )?)?,
    })
}

pub async fn start_listening(http_config: HttpServerConfig, state: Arc<AppState>) -> Result<()> {
    let mut root_store = RootCertStore::empty();
    root_store.add(CertificateDer::from_pem_slice(
        &http_config.client_ca_bytes,
    )?)?;

    let client_verifier = WebPkiClientVerifier::builder(Arc::new(root_store)).build()?;

    let tls_config = ServerConfig::builder()
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(
            vec![CertificateDer::from_pem_slice(&http_config.cert_bytes)?],
            PrivateKeyDer::from_pem_slice(&http_config.key_bytes)?,
        )?;

    let acceptor = TlsAcceptor::from(Arc::new(tls_config));

    let listener: TcpListener = TcpListener::bind(&http_config.listen_on).await?;
    log::info!("http tcp server listening on {}", &http_config.listen_on);

    loop {
        let (mut socket, sender) = listener.accept().await?;
        log::debug!("http tcp server received connection from {sender}");

        let state = state.clone();
        let acceptor = acceptor.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_conn(&mut socket, state, acceptor).await {
                log::warn!("{:?}", err.context("Failed to handle http tcp connection"))
            }

            let _ = socket.shutdown().await;
        });
    }
}
