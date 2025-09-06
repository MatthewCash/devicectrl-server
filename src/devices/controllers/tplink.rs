use anyhow::{Context, Result, bail};
use devicectrl_common::{
    DeviceState, DeviceStateUpdate, DeviceType, UpdateNotification, UpdateRequest,
    device_types::switch::SwitchState,
};
use serde_derive::Deserialize;
use serde_json::{Value, json};
use std::{net::Ipv4Addr, time::Duration};
use tokio::{net::UdpSocket, time::timeout};

use crate::{
    AppState,
    devices::{Device, Devices, dispatch::process_update_notification},
};

use super::ControllerConfig;

const ENCRYPTION_KEY: u8 = 0xAB;

pub fn encrypt(message: Vec<u8>) -> Vec<u8> {
    let mut key = ENCRYPTION_KEY;

    message
        .iter()
        .map(|b| {
            key ^= b;
            key
        })
        .collect()
}

pub fn decrypt(cipher: Vec<u8>) -> Vec<u8> {
    let mut key = ENCRYPTION_KEY;

    cipher
        .iter()
        .map(|b| {
            let prev_key = key;
            key = *b;
            b ^ prev_key
        })
        .collect()
}

#[derive(Clone, Debug, Deserialize)]
pub struct TplinkControllerConfig {
    address: Ipv4Addr,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TplinkControllerGlobalConfig {
    response_timeout: Duration,
}

fn build_update_payload(state: &DeviceStateUpdate) -> Result<Option<Value>> {
    Ok(match state {
        DeviceStateUpdate::Switch(state) => state.power.map(|power| {
            json!({
                "system": {
                    "set_relay_state": {
                        "state": if power { 1 } else { 0 }
                    }
                }
            })
        }),
        _ => bail!("Unsupported state for tplink controller!"),
    })
}

fn build_query_payload() -> Value {
    json! ({
        "system": {
            "get_sysinfo": {}
        }
    })
}

fn parse_query_payload(payload: &Value, device_type: &DeviceType) -> Result<DeviceState> {
    Ok(match device_type {
        DeviceType::Switch => DeviceState::Switch(SwitchState {
            power: payload["system"]["get_sysinfo"]["relay_state"]
                .as_i64()
                .context("tplink device query payload missing relay state")?
                == 1,
        }),
        _ => bail!("Unsupported state for tplink controller!"),
    })
}

fn is_response_success(payload: &Value, device_type: &DeviceType) -> Result<bool> {
    Ok(match device_type {
        DeviceType::Switch => {
            payload["system"]["set_relay_state"]["err_code"]
                .as_i64()
                .context("tplink device response payload missing relay state err code")?
                == 0
        }
        _ => bail!("Unsupported state for tplink controller!"),
    })
}

pub struct TplinkController {
    global_config: TplinkControllerGlobalConfig,
    socket: UdpSocket,
}

impl TplinkController {
    pub async fn new(global_config: TplinkControllerGlobalConfig) -> Result<Self> {
        Ok(Self {
            global_config,
            socket: UdpSocket::bind("0.0.0.0:0").await?,
        })
    }
    pub async fn start_listening(&self, devices: &'static Devices, app_state: &AppState) {
        loop {
            if let Err(err) = async {
                let mut buf = vec![0u8; 1500];
                let (size, addr) = self.socket.recv_from(&mut buf).await?;
                buf.truncate(size);

                let devices = devices.read().await;
                let Some(device) = devices.values().find(|device| matches!(&device.controller, ControllerConfig::Tplink(config) if config.address == addr.ip())) else {
                    log::warn!("received state for unknown device [{}]", addr.ip());
                    return Ok(());
                };

                let state = parse_query_payload(
                    &serde_json::from_slice(&decrypt(buf))?,
                    &device.device_type,
                )?;

                process_update_notification(UpdateNotification {
                    device_id: device.id,
                    reachable: true,
                    new_state: state,
                }, app_state).context("failed to process tplink update notification")?;

                Result::<()>::Ok(())
            }
            .await
            {
                log::error!("{:?}", err.context("failed to receive message"));
            }
        }
    }

    async fn send_command(
        &self,
        socket: &UdpSocket,
        config: &TplinkControllerConfig,
        command: &Value,
    ) -> Result<()> {
        let data = serde_json::to_vec(command)?;

        socket
            .send_to(&encrypt(data), (config.address, 9999))
            .await?;

        Ok(())
    }
    async fn receive_response(&self, sock: &UdpSocket) -> Result<Value> {
        let mut buf = vec![0u8; 1500];
        let size = sock.recv(&mut buf).await?;
        buf.truncate(size);

        Ok(serde_json::from_slice(&decrypt(buf))?)
    }

    pub async fn send_query(
        &self,
        config: &TplinkControllerConfig,
        _device: &Device,
    ) -> Result<()> {
        self.send_command(&self.socket, config, &build_query_payload())
            .await
    }
    pub async fn send_update(
        &self,
        config: &TplinkControllerConfig,
        device: &Device,
        update: &UpdateRequest,
    ) -> Result<()> {
        let Some(payload) = build_update_payload(&update.change_to)? else {
            return Ok(());
        };

        let socket = UdpSocket::bind("0.0.0.0:0").await?;

        self.send_command(&socket, config, &payload).await?;

        let set_res = timeout(
            self.global_config.response_timeout,
            self.receive_response(&socket),
        )
        .await
        .context("timed out waiting for response")??;

        if !is_response_success(&set_res, &device.device_type)? {
            bail!("Device update was unsuccessful");
        }

        self.send_query(config, device).await
    }
}
