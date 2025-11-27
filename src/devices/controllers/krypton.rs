use anyhow::{Context, Result, anyhow, bail};
use devicectrl_common::{
    DeviceId, UpdateCommand, UpdateRequest,
    protocol::krypton::{DeviceBoundKryptonMessage, ServerBoundKryptonMessage},
};
use futures::{
    SinkExt, TryStreamExt,
    future::{Either, select},
};
use serde_derive::Deserialize;
use std::{collections::HashMap, net::SocketAddr, pin::pin, sync::Arc};
use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
    sync::broadcast,
};
use tokio_rustls::{
    TlsAcceptor,
    rustls::{
        RootCertStore, ServerConfig,
        pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
        server::{Acceptor, WebPkiClientVerifier},
    },
};
use tokio_util::{
    bytes::BytesMut,
    codec::{Framed, LengthDelimitedCodec},
};

use crate::{
    AppState,
    config::deserialize_file_path_bytes,
    devices::{Device, Devices, dispatch::process_update_notification},
};

use super::ControllerConfig;

#[derive(Clone, Debug, Deserialize)]
pub struct KryptonControllerConfig {
    #[serde(rename = "cert_path", deserialize_with = "deserialize_file_path_bytes")]
    cert_bytes: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct KryptonControllerGlobalConfig {
    listen_on: SocketAddr,

    #[serde(rename = "cert_path", deserialize_with = "deserialize_file_path_bytes")]
    cert_bytes: Vec<u8>,
    #[serde(rename = "key_path", deserialize_with = "deserialize_file_path_bytes")]
    key_bytes: Vec<u8>,
}

fn get_tls_config(
    device_certs: &HashMap<DeviceId, Vec<u8>>,
    global_config: &KryptonControllerGlobalConfig,
    device_id: &DeviceId,
) -> Result<Arc<ServerConfig>> {
    let device_cert_bytes = device_certs
        .get(device_id)
        .context("Device certificate not found")?;

    // Create a client certificate verifier using the device's certificate
    let mut root_store = RootCertStore::empty();
    let device_cert = CertificateDer::from_pem_slice(device_cert_bytes)
        .map_err(|e| anyhow!("Failed to parse device certificate: {}", e))?;
    root_store
        .add(device_cert.clone())
        .map_err(|e| anyhow!("Failed to add device cert to root store: {}", e))?;

    let client_verifier = WebPkiClientVerifier::builder(Arc::new(root_store))
        .build()
        .map_err(|e| anyhow!("Failed to build client verifier: {}", e))?;

    // Use the global server certificate and key
    let server_cert = CertificateDer::from_pem_slice(&global_config.cert_bytes)
        .map_err(|e| anyhow!("Failed to parse server certificate: {}", e))?;

    let server_key = PrivateKeyDer::from_pem_slice(&global_config.key_bytes)
        .map_err(|e| anyhow!("Failed to parse server private key: {}", e))?;

    let config = ServerConfig::builder()
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(vec![server_cert], server_key)
        .map_err(|e| anyhow!("Failed to build TLS config: {}", e))?;

    Ok(Arc::new(config))
}

async fn handle_conn(
    socket: &mut TcpStream,
    devices: &'static Devices,
    mut request_receiver: broadcast::Receiver<DeviceBoundKryptonMessage>,
    app_state: &AppState,
    global_config: &KryptonControllerGlobalConfig,
    device_certs: &HashMap<DeviceId, Vec<u8>>,
) -> Result<()> {
    let mut acceptor = Acceptor::default();

    let accepted = loop {
        let mut ch_buf = [0u8; 4096];
        let bytes_read = socket.try_read(&mut ch_buf)?;
        acceptor.read_tls(&mut &ch_buf[..bytes_read])?;

        if let Some(accepted) = acceptor
            .accept()
            .map_err(|(e, _a)| e)
            .context("failed to accept krypton tls connection")?
        {
            break accepted;
        }
    };

    let hello = accepted.client_hello();
    let device_id = DeviceId::from(
        hello
            .server_name()
            .and_then(|s| s.strip_prefix("krypton-deviceid="))
            .context("missing device id in SNI")?,
    )
    .map_err(|_| anyhow!("invalid device id"))?;

    let tls_config = get_tls_config(device_certs, global_config, &device_id)
        .context("failed to get tls config for device")?;

    log::info!(
        "Krypton device [{device_id}] connected from {:?}",
        socket.peer_addr()
    );

    let mut stream = Framed::new(
        TlsAcceptor::from(tls_config).accept(socket).await?,
        LengthDelimitedCodec::new(),
    );

    loop {
        match select(stream.try_next(), pin!(request_receiver.recv())).await {
            Either::Left((buf, _)) => {
                let Some(buf) = buf? else { return Ok(()) };
                log::debug!("received message from device");

                if let Err(err) = handle_message(&buf, &device_id, devices, app_state)
                    .await
                    .context("failed to handle krypton message")
                {
                    log::warn!("{err:?}");

                    stream
                        .send(serde_json::to_vec::<DeviceBoundKryptonMessage>(&err.into())?.into())
                        .await?;
                }
            }
            Either::Right((request, _)) => {
                let request = request.context("failed to recv krypton message")?;

                if Some(device_id)
                    != match &request {
                        DeviceBoundKryptonMessage::UpdateCommand(command) => {
                            Some(command.device_id)
                        }
                        DeviceBoundKryptonMessage::StateQuery { device_id } => Some(*device_id),
                        _ => None,
                    }
                {
                    continue;
                }

                let data = serde_json::to_vec(&request)?;

                log::debug!("sending request to device");
                stream.send(data.into()).await?;
            }
        }
    }
}

async fn handle_message(
    buf: &BytesMut,
    device_id: &DeviceId,
    devices: &Devices,
    app_state: &AppState,
) -> Result<()> {
    {
        let devices = devices.read().await;

        let ControllerConfig::Krypton(ref _config) = devices
            .get(device_id)
            .context("Message received from unknown device")?
            .controller
        else {
            bail!("Device is not a krypton device");
        };
    }

    let message: ServerBoundKryptonMessage = serde_json::from_slice(buf)?;

    match message {
        ServerBoundKryptonMessage::Identify(_) => {
            bail!("Device sent another identify")
        }
        ServerBoundKryptonMessage::RequestReceived => {}
        ServerBoundKryptonMessage::Failure(msg) => {
            log::error!("Device failure: {msg:?}")
        }
        ServerBoundKryptonMessage::UpdateNotification(notification) => {
            if notification.device_id != *device_id {
                bail!("Device sent update with incorrect device id");
            }

            process_update_notification(notification, app_state)
                .await
                .context("failed to process krypton update notification")?;
        }
        _ => log::warn!("Device sent unknown message"),
    }

    Ok(())
}

pub struct KryptonController {
    global_config: KryptonControllerGlobalConfig,
    listener: TcpListener,
    request_sender: broadcast::Sender<DeviceBoundKryptonMessage>,
    request_receiver: broadcast::Receiver<DeviceBoundKryptonMessage>,
}

impl KryptonController {
    pub async fn new(global_config: KryptonControllerGlobalConfig) -> Result<Self> {
        let (sender, receiver) = broadcast::channel(64);

        Ok(Self {
            listener: TcpListener::bind(&global_config.listen_on).await?,
            global_config,
            request_sender: sender,
            request_receiver: receiver,
        })
    }

    pub async fn start_listening(
        &self,
        devices: &'static Devices,
        app_state: &'static AppState,
    ) -> Result<()> {
        let device_certs = {
            let certs = Box::leak(Box::new(HashMap::<DeviceId, Vec<u8>>::new()));

            for device in devices.read().await.values() {
                if let ControllerConfig::Krypton(config) = &device.controller {
                    certs.insert(device.id, config.cert_bytes.clone());
                }
            }

            &*certs // make immutable reference
        };

        loop {
            let (mut socket, sender) = self.listener.accept().await?;
            log::debug!("krypton controller received connection from {sender}");

            let request_receiver = self.request_receiver.resubscribe();
            let global_config = self.global_config.clone();

            tokio::spawn(async move {
                if let Err(err) = handle_conn(
                    &mut socket,
                    devices,
                    request_receiver,
                    app_state,
                    &global_config,
                    device_certs,
                )
                .await
                .context("Failed to handle krypton controller connection")
                {
                    log::warn!("{err:?}")
                }

                let _ = socket.shutdown().await;
            });
        }
    }

    pub async fn send_query(
        &self,
        _config: &KryptonControllerConfig,
        device: &Device,
    ) -> Result<()> {
        self.request_sender
            .send(DeviceBoundKryptonMessage::StateQuery {
                device_id: device.id,
            })?;

        Ok(())
    }

    pub async fn send_update(
        &self,
        _config: &KryptonControllerConfig,
        _device: &Device,
        request: &UpdateRequest,
    ) -> Result<()> {
        self.request_sender
            .send(DeviceBoundKryptonMessage::UpdateCommand(UpdateCommand {
                device_id: request.device_id,
                update: request.update,
            }))?;

        Ok(())
    }
}
