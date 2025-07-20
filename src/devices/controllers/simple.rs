use anyhow::{Context, Result, bail};
use devicectrl_common::{
    DeviceId, UpdateCommand, UpdateRequest,
    protocol::simple::{DeviceBoundSimpleMessage, SIGNATURE_LEN, ServerBoundSimpleMessage},
};
use futures::{
    SinkExt, TryStreamExt,
    future::{Either, select},
};
use p256::ecdsa::{
    SigningKey, VerifyingKey,
    signature::{SignerMut, Verifier},
};
use p256::{ecdsa::Signature, pkcs8::DecodePublicKey};
use serde::{Deserialize, de};
use serde_derive::Deserialize;
use std::{net::SocketAddr, pin::pin, sync::Arc};
use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
    sync::broadcast,
};
use tokio_util::{
    bytes::BytesMut,
    codec::{Framed, LengthDelimitedCodec},
};

use crate::{
    AppState,
    auth::deserialize_signing_key,
    devices::{Device, Devices, dispatch::process_update_notification},
};

use super::ControllerConfig;

#[derive(Clone, Debug, Deserialize)]
pub struct SimpleControllerConfig {
    #[serde(
        rename = "public_key_path",
        deserialize_with = "deserialize_verifying_key"
    )]
    public_key: VerifyingKey,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SimpleControllerGlobalConfig {
    #[serde(
        rename = "server_private_key_path",
        deserialize_with = "deserialize_signing_key"
    )]
    server_private_key: SigningKey,
    listen_on: SocketAddr,
}

fn deserialize_verifying_key<'de, D>(deserializer: D) -> Result<VerifyingKey, D::Error>
where
    D: de::Deserializer<'de>,
{
    let der_bytes = std::fs::read(String::deserialize(deserializer)?).map_err(de::Error::custom)?;
    VerifyingKey::from_public_key_der(&der_bytes).map_err(de::Error::custom)
}

async fn handle_conn(
    socket: &mut TcpStream,
    devices: Devices,
    mut request_receiver: broadcast::Receiver<DeviceBoundSimpleMessage>,
    server_private_key: &SigningKey,
    app_state: Arc<AppState>,
) -> Result<()> {
    let mut stream = Framed::new(socket, LengthDelimitedCodec::new());

    let Some(buf) = stream.try_next().await? else {
        return Ok(());
    };

    let message: ServerBoundSimpleMessage = serde_json::from_slice(&buf)?;

    let ServerBoundSimpleMessage::Identify(device_id) = message else {
        bail!("Device did not identify itself!");
    };
    if !devices.read().await.contains_key(&device_id) {
        bail!("Device attempted to identify with unknown id");
    }

    log::info!(
        "Simple device [{device_id}] connected from {:?}",
        stream.get_ref().peer_addr()
    );

    loop {
        match select(stream.try_next(), pin!(request_receiver.recv())).await {
            Either::Left((buf, _)) => {
                let Some(buf) = buf? else { return Ok(()) };
                log::debug!("received message from device");

                if let Err(err) = handle_message(&buf, &device_id, devices.clone(), &app_state)
                    .await
                    .context("failed to handle simple message")
                {
                    log::warn!("{:?}", err);

                    stream
                        .send(serde_json::to_vec::<DeviceBoundSimpleMessage>(&err.into())?.into())
                        .await?;
                }
            }
            Either::Right((request, _)) => {
                let request = request.context("failed to recv simple message")?;

                if Some(device_id)
                    != match &request {
                        DeviceBoundSimpleMessage::UpdateCommand(command) => Some(command.device_id),
                        DeviceBoundSimpleMessage::StateQuery { device_id } => Some(*device_id),
                        _ => None,
                    }
                {
                    continue;
                }

                let mut data = serde_json::to_vec(&request)?;

                let sig: Signature = server_private_key.clone().try_sign(&data)?;
                data.splice(0..0, sig.to_bytes());

                log::debug!("sending request to device");
                stream.send(data.into()).await?;
            }
        }
    }
}

async fn handle_message(
    buf: &BytesMut,
    device_id: &DeviceId,
    devices: Devices,
    app_state: &AppState,
) -> Result<()> {
    let sig: &[u8; SIGNATURE_LEN] = &buf
        .get(..SIGNATURE_LEN)
        .context("message is not long enough for signature")?
        .try_into()?;
    let data = &buf.get(SIGNATURE_LEN..).context("message is too short")?;

    let devices = devices.read().await;

    let ControllerConfig::Simple(ref config) = devices
        .get(device_id)
        .context("Message received from unknown device")?
        .controller
    else {
        bail!("Device is not a simple device");
    };

    config
        .public_key
        .verify(data, &Signature::from_slice(sig)?)?;

    let message: ServerBoundSimpleMessage = serde_json::from_slice(data)?;

    match message {
        ServerBoundSimpleMessage::Identify(_) => {
            bail!("Device sent another identify")
        }
        ServerBoundSimpleMessage::RequestReceived => {}
        ServerBoundSimpleMessage::Failure(msg) => {
            log::error!("Device failure: {:?}", msg)
        }
        ServerBoundSimpleMessage::UpdateNotification(notification) => {
            if notification.device_id != *device_id {
                bail!("Device sent update with incorrect device id");
            }

            process_update_notification(notification, app_state)
                .context("failed to process simple update notification")?;
        }
        _ => log::warn!("Device sent unknown message"),
    }

    Ok(())
}

pub struct SimpleController {
    global_config: SimpleControllerGlobalConfig,
    listener: TcpListener,
    request_sender: broadcast::Sender<DeviceBoundSimpleMessage>,
    request_receiver: broadcast::Receiver<DeviceBoundSimpleMessage>,
}

impl SimpleController {
    pub async fn new(global_config: SimpleControllerGlobalConfig) -> Result<Self> {
        let (sender, receiver) = broadcast::channel(64);
        Ok(Self {
            listener: TcpListener::bind(&global_config.listen_on).await?,
            global_config,
            request_sender: sender,
            request_receiver: receiver,
        })
    }
    pub async fn start_listening(&self, devices: Devices, app_state: Arc<AppState>) {
        loop {
            let (mut socket, sender) = self.listener.accept().await.unwrap();
            log::debug!("simple controller received connection from {sender}");

            let devices = devices.clone();
            let request_receiver = self.request_receiver.resubscribe();
            let server_private_key = self.global_config.server_private_key.clone();
            let app_state = app_state.clone();
            tokio::spawn(async move {
                if let Err(err) = handle_conn(
                    &mut socket,
                    devices,
                    request_receiver,
                    &server_private_key,
                    app_state,
                )
                .await
                {
                    log::warn!(
                        "{:?}",
                        err.context("Failed to handle simple controller connection")
                    )
                }

                let _ = socket.shutdown().await;
            });
        }
    }

    pub async fn send_query(
        &self,
        _config: &SimpleControllerConfig,
        device: &Device,
    ) -> Result<()> {
        self.request_sender
            .send(DeviceBoundSimpleMessage::StateQuery {
                device_id: device.id,
            })?;

        Ok(())
    }
    pub async fn send_update(
        &self,
        _config: &SimpleControllerConfig,
        _device: &Device,
        update: &UpdateRequest,
    ) -> Result<()> {
        self.request_sender
            .send(DeviceBoundSimpleMessage::UpdateCommand(UpdateCommand {
                device_id: update.device_id,
                change_to: update.change_to.clone(),
            }))?;

        Ok(())
    }
}
