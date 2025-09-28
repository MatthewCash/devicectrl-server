use anyhow::{Context, Result, bail};
use devicectrl_common::{
    DeviceId, UpdateCommand, UpdateRequest,
    protocol::simple::{DeviceBoundSimpleMessage, SIGNATURE_LEN, ServerBoundSimpleMessage},
};
use futures::future::{Either, select};
use p256::{ecdsa::Signature, elliptic_curve::rand_core::OsRng};
use p256::{
    ecdsa::{
        SigningKey, VerifyingKey,
        signature::{SignerMut, Verifier},
    },
    elliptic_curve::rand_core::RngCore,
};
use serde_derive::Deserialize;
use std::{net::SocketAddr, pin::pin};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{broadcast, mpsc},
};
use tokio_util::bytes::BytesMut;

use crate::{
    AppState,
    config::{deserialize_signing_key, deserialize_verifying_key},
    devices::ControllerConfig,
    devices::{Device, Devices, dispatch::process_update_notification},
};

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

async fn read_signed_message(
    mut socket: impl AsyncRead + Unpin,
    expected_client_nonce: &mut u32,
    public_key: &VerifyingKey,
) -> Result<ServerBoundSimpleMessage> {
    *expected_client_nonce = expected_client_nonce.wrapping_add(1);
    let client_nonce = socket
        .read_u32()
        .await
        .context("Client nonce is not 4 bytes")?;

    if *expected_client_nonce != client_nonce {
        bail!(
            "Client nonce mismatch: expected {}, got {}",
            expected_client_nonce,
            client_nonce
        );
    }

    let data_len = socket
        .read_u32()
        .await
        .context("Data length is not 4 bytes")? as usize;

    // Prepare buffer for nonce + length + message + signature
    let nonce_len = size_of_val(&client_nonce);
    let length_len = size_of_val(&data_len);
    let total_len = nonce_len + length_len + data_len + SIGNATURE_LEN;
    let mut buf = vec![0u8; total_len];

    // Copy nonce and length into buffer so that it can be verified by the signature
    buf[..nonce_len].copy_from_slice(&client_nonce.to_be_bytes());
    buf[nonce_len..nonce_len + length_len].copy_from_slice(&data_len.to_be_bytes());

    // Read remaining message + signature
    socket
        .read_exact(&mut buf[nonce_len + length_len..])
        .await
        .context("Failed to read message")?;

    let data_end = nonce_len + length_len + data_len;
    let data = &buf[..data_end];
    let sig = &buf[data_end..];

    public_key.verify(data, &Signature::from_slice(sig)?)?;

    Ok(serde_json::from_slice(
        &buf[nonce_len + length_len..data_end],
    )?)
}

fn create_signed_message(
    message: &DeviceBoundSimpleMessage,
    server_nonce: &mut u32,
    server_private_key: &SigningKey,
) -> Result<Vec<u8>> {
    let message_data = serde_json::to_vec(message)?;
    let data_len = message_data.len() as u32;

    let mut data = Vec::with_capacity(
        size_of_val(server_nonce) + size_of_val(&data_len) + message_data.len() + SIGNATURE_LEN,
    );

    *server_nonce = server_nonce.wrapping_add(1);
    data.extend_from_slice(&server_nonce.to_be_bytes());
    data.extend_from_slice(&data_len.to_be_bytes());
    data.extend_from_slice(&message_data);

    let sig: Signature = server_private_key.clone().try_sign(&data)?;

    data.extend_from_slice(&sig.to_bytes());

    Ok(data)
}

async fn handle_conn(
    socket: &mut TcpStream,
    devices: &'static Devices,
    mut request_receiver: broadcast::Receiver<DeviceBoundSimpleMessage>,
    server_private_key: &SigningKey,
    app_state: &AppState,
) -> Result<()> {
    // the client must include an incremented nonce in all messages
    let mut server_nonce = OsRng.next_u32();
    socket.write_u32(server_nonce).await?;

    // the client sends its own nonce
    let mut expected_client_nonce = socket
        .read_u32()
        .await
        .context("Client nonce is not 4 bytes")?;

    let identify_message_len = socket
        .read_u32()
        .await
        .context("Identify message length is not 4 bytes")? as usize;

    let mut identify_buf = BytesMut::with_capacity(identify_message_len);
    socket
        .read_exact(&mut identify_buf)
        .await
        .context("Failed to read identify message")?;
    let message: ServerBoundSimpleMessage = serde_json::from_slice(&identify_buf)?;

    let ServerBoundSimpleMessage::Identify(device_id) = message else {
        bail!("Device did not identify itself!");
    };

    // Now that we know the device ID, we can get its public key
    let public_key = {
        let devices = devices.read().await;
        let ControllerConfig::Simple(ref config) = devices
            .get(&device_id)
            .context("Message received from unknown device")?
            .controller
        else {
            bail!("Device is not a simple device");
        };

        config.public_key
    };

    log::info!(
        "Simple device [{device_id}] connected from {:?}",
        socket.peer_addr()
    );

    let (mut read, mut write) = socket.split();

    let (sender, mut receiver) = mpsc::channel::<ServerBoundSimpleMessage>(16);

    moro::async_scope!(|scope| {
        // read calls are done concurrently so that they are not cancelled by select()
        scope.spawn(async {
            loop {
                match read_signed_message(&mut read, &mut expected_client_nonce, &public_key).await
                {
                    Ok(message) => {
                        if let Err(err) = sender.send(message).await {
                            log::warn!("Failed to send message to handler: {err:?}");
                            break;
                        }
                    }
                    Err(err) => {
                        log::warn!("Failed to read signed message: {err:?}");
                        break;
                    }
                }
            }
        });

        loop {
            match select(pin!(receiver.recv()), pin!(request_receiver.recv())).await {
                Either::Left((message, _)) => {
                    let Some(message) = message else {
                        return Ok(());
                    };
                    log::debug!("received message from device");

                    if let Err(err) = handle_message(&message, &device_id, app_state)
                        .await
                        .context("failed to handle simple message")
                    {
                        log::warn!("{err:?}");

                        write
                            .write_all(&create_signed_message(
                                &err.into(),
                                &mut server_nonce,
                                server_private_key,
                            )?)
                            .await?;
                    }
                }
                Either::Right((request, _)) => {
                    let request = request.context("failed to recv simple message")?;

                    if Some(device_id)
                        != match &request {
                            DeviceBoundSimpleMessage::UpdateCommand(command) => {
                                Some(command.device_id)
                            }
                            DeviceBoundSimpleMessage::StateQuery { device_id } => Some(*device_id),
                            _ => None,
                        }
                    {
                        continue;
                    }

                    log::debug!("sending request to device");
                    write
                        .write_all(&create_signed_message(
                            &request,
                            &mut server_nonce,
                            server_private_key,
                        )?)
                        .await?;
                }
            }
        }
    })
    .await
}

async fn handle_message(
    message: &ServerBoundSimpleMessage,
    device_id: &DeviceId,
    app_state: &AppState,
) -> Result<()> {
    match message {
        ServerBoundSimpleMessage::Identify(_) => {
            bail!("Device sent another identify")
        }
        ServerBoundSimpleMessage::RequestReceived => {}
        ServerBoundSimpleMessage::Failure(msg) => {
            log::error!("Device failure: {msg:?}")
        }
        ServerBoundSimpleMessage::UpdateNotification(notification) => {
            if notification.device_id != *device_id {
                bail!("Device sent update with incorrect device id");
            }

            process_update_notification(*notification, app_state)
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
    pub async fn start_listening(
        &self,
        devices: &'static Devices,
        app_state: &'static AppState,
    ) -> Result<()> {
        loop {
            let (mut socket, sender) = self.listener.accept().await?;

            log::debug!("simple controller received connection from {sender}");

            let request_receiver = self.request_receiver.resubscribe();
            let server_private_key = self.global_config.server_private_key.clone();

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
        request: &UpdateRequest,
    ) -> Result<()> {
        self.request_sender
            .send(DeviceBoundSimpleMessage::UpdateCommand(UpdateCommand {
                device_id: request.device_id,
                update: request.update,
            }))?;

        Ok(())
    }
}
