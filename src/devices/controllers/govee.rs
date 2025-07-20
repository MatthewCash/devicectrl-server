use anyhow::{Context, Result, bail};
use devicectrl_common::{
    DeviceState, DeviceStateUpdate, DeviceType, UpdateNotification, UpdateRequest,
    device_types::led_strip::LedStripState,
};
use serde_derive::Deserialize;
use serde_json::{Value, json};
use std::{net::Ipv4Addr, sync::Arc, time::Duration};
use tokio::{net::UdpSocket, time::sleep};

use crate::{
    AppState,
    devices::{Device, Devices, dispatch::process_update_notification},
};

use super::ControllerConfig;

#[derive(Clone, Debug, Deserialize)]
pub struct GoveeControllerConfig {
    address: Ipv4Addr,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GoveeControllerGlobalConfig {
    update_query_delay: Duration,
}

const GOVEE_SEND_PORT: u16 = 4003;
const GOVEE_LISTEN_PORT: u16 = 4002;

fn build_update_payload(state: &DeviceStateUpdate) -> Result<Option<Value>> {
    Ok(match state {
        DeviceStateUpdate::LedStrip(state) => {
            if state.power.is_some_and(|p| !p) {
                Some(json!({
                    "msg": {
                        "cmd": "turn",
                        "data": {
                            "value": 0,
                        }
                    }
                }))
            } else if let Some(brightness) = state.brightness {
                // if brightness is set, we can use that to turn the device on
                Some(json!({
                    "msg": {
                        "cmd": "brightness",
                        "data": {
                            "value": brightness,
                        }
                    }
                }))
            } else if state.power.is_some_and(|p| p) {
                Some(json!({
                    "msg": {
                        "cmd": "turn",
                        "data": {
                            "value": 1,
                        }
                    }
                }))
            } else {
                None
            }
        }
        _ => bail!("Unsupported state for govee controller!"),
    })
}

#[allow(dead_code)]
fn build_query_payload() -> Value {
    json! ({
        "msg":{
            "cmd": "devStatus",
            "data": {}
        }
    })
}

fn parse_query_payload(payload: &Value, device_type: &DeviceType) -> Result<DeviceState> {
    Ok(match device_type {
        DeviceType::LedStrip => DeviceState::LedStrip(LedStripState {
            power: payload["msg"]["data"]["onOff"]
                .as_i64()
                .context("govee device query payload missing onOff")?
                == 1,
            brightness: payload["msg"]["data"]["brightness"]
                .as_i64()
                .context("govee device query payload missing brightness")?
                .try_into()?,
        }),
        _ => bail!("Unsupported state for govee controller!"),
    })
}

pub struct GoveeController {
    #[allow(dead_code)]
    global_config: GoveeControllerGlobalConfig,
    socket: UdpSocket,
}

impl GoveeController {
    pub async fn new(global_config: GoveeControllerGlobalConfig) -> Result<Self> {
        Ok(Self {
            global_config,
            socket: UdpSocket::bind(("0.0.0.0", GOVEE_LISTEN_PORT)).await?,
        })
    }
    pub async fn start_listening(&self, devices: Devices, app_state: Arc<AppState>) {
        loop {
            if let Err(err) = async {
                let mut buf = [0u8; 1500];
                let (size, addr) = self.socket.recv_from(&mut buf).await?;

                let devices = devices.read().await;
                let Some(device) = devices.values().find(|device| matches!(&device.controller, ControllerConfig::Govee(config) if config.address == addr.ip())) else {
                    log::warn!("received state for unknown device [{}]", addr.ip());
                    return Ok(());
                };

                let state = parse_query_payload(
                    &serde_json::from_slice(&buf[..size])?,
                    &device.device_type,
                )?;

                process_update_notification(UpdateNotification {
                    device_id: device.id,
                    reachable: true,
                    new_state: state,
                }, &app_state).context("failed to process govee update notification")?;

                Result::<()>::Ok(())
            }
            .await
            {
                log::error!("{:?}", err.context("failed to receive message"));
            }
        }
    }

    async fn send_command(&self, config: &GoveeControllerConfig, command: &Value) -> Result<()> {
        let data = serde_json::to_vec(command)?;

        self.socket
            .send_to(&data, (config.address, GOVEE_SEND_PORT))
            .await?;

        Ok(())
    }
    pub async fn send_query(&self, config: &GoveeControllerConfig, _device: &Device) -> Result<()> {
        self.send_command(config, &build_query_payload()).await
    }
    pub async fn send_update(
        &self,
        config: &GoveeControllerConfig,
        _device: &Device,
        update: &UpdateRequest,
    ) -> Result<()> {
        let Some(payload) = build_update_payload(&update.change_to)? else {
            return Ok(());
        };

        self.send_command(config, &payload).await?;

        sleep(self.global_config.update_query_delay).await;

        self.send_command(config, &build_query_payload()).await
    }
}
