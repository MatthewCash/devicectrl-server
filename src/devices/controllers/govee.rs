use anyhow::{Context, Result, bail};
use devicectrl_common::{
    DeviceState, DeviceType, UpdateNotification, UpdateRequest,
    device_types::{NumericProperties, dimmable_light::DimmableLightState, switch::SwitchPower},
    updates::AttributeUpdate,
};
use serde_derive::Deserialize;
use serde_json::{Value, json};
use std::{net::Ipv4Addr, time::Duration};
use tokio::{net::UdpSocket, time::sleep};

use crate::{
    AppState,
    devices::{Device, Devices, dispatch::process_update_notification},
};

use super::ControllerConfig;

const BRIGHTNESS_PROPS: NumericProperties = NumericProperties {
    min: 0,
    max: 100,
    step: 1,
};

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

fn build_update_payload(device: &Device, update: &AttributeUpdate) -> Result<Value> {
    let brightness_state = match device.state {
        DeviceState::Unknown => BRIGHTNESS_PROPS.to_state(0),
        DeviceState::DimmableLight(state) => state.brightness,
        DeviceState::ColorLight(state) => state.brightness,
        _ => bail!("Unsupported state for govee controller update!"),
    };

    Ok(match update {
        AttributeUpdate::Power(power) => json!({
            "msg": {
                "cmd": "turn",
                "data": {
                    "value": match power {
                        SwitchPower::On => 1,
                        SwitchPower::Off => 0,
                    }
                }
            }
        }),
        // brightness will also turn the device on if it is off
        AttributeUpdate::Brightness(brightness) => json!({
            "msg": {
                "cmd": "brightness",
                "data": {
                    "value": brightness.apply_to(&brightness_state),
                }
            }
        }),
        _ => bail!("Unsupported update for govee controller update!"),
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
        DeviceType::DimmableLight => DeviceState::DimmableLight(DimmableLightState {
            power: match payload["msg"]["data"]["onOff"]
                .as_i64()
                .context("govee device query payload missing onOff")?
            {
                1 => SwitchPower::On,
                _ => SwitchPower::Off,
            },
            // govee devices report brightness as integer 0-100
            brightness: BRIGHTNESS_PROPS.to_state(
                payload["msg"]["data"]["brightness"]
                    .as_u64()
                    .context("govee device query payload missing brightness")?
                    as u32,
            ),
        }),
        _ => bail!("Unsupported state for govee controller query!"),
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
    pub async fn start_listening(
        &self,
        devices: &'static Devices,
        app_state: &AppState,
    ) -> Result<()> {
        loop {
            if let Err(err) = async {
                let mut buf = [0u8; 1500];
                let (size, addr) = self.socket.recv_from(&mut buf).await?;


                let notification = {
                    let devices = devices.read().await;

                    let Some(device) = devices.values().find(|device| matches!(&device.controller, ControllerConfig::Govee(config) if config.address == addr.ip())) else {
                        log::warn!("received state for unknown device [{}]", addr.ip());
                        return Ok(());
                    };

                    let state = parse_query_payload(
                        &serde_json::from_slice(&buf[..size])?,
                        &device.device_type,
                    )?;

                    UpdateNotification {
                        device_id: device.id,
                        reachable: true,
                        new_state: state,
                    }
                };

                process_update_notification(notification, app_state).await.context("failed to process govee update notification")?;

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
        device: &Device,
        request: &UpdateRequest,
    ) -> Result<()> {
        let payload = build_update_payload(device, &request.update)?;

        self.send_command(config, &payload).await?;

        sleep(self.global_config.update_query_delay).await;

        self.send_command(config, &build_query_payload()).await
    }
}
