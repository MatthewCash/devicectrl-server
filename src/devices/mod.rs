use anyhow::Result;
use controllers::ControllerConfig;
use devicectrl_common::DeviceId;
use devicectrl_common::{DeviceState, DeviceType};
use std::collections::HashMap;

use serde_derive::Deserialize;
use tokio::sync::RwLock;

use crate::config::Config;

pub mod controllers;
pub mod dispatch;

#[derive(Clone, Debug, Deserialize)]
pub struct DeviceConfig {
    device_type: DeviceType,
    controller: ControllerConfig,
}

pub struct Device {
    id: DeviceId,
    device_type: DeviceType,
    #[allow(dead_code)]
    state: DeviceState,
    controller: ControllerConfig,
}

pub type DevicesConfig = HashMap<String, DeviceConfig>;

pub type Devices = RwLock<HashMap<DeviceId, Device>>;

pub async fn load_devices(config: &Config) -> Result<&'static Devices> {
    let devices = config
        .devices
        .iter()
        .map(|(config_id, device_config)| {
            let mut id = DeviceId::new();
            id.push_str(config_id);

            (
                id,
                Device {
                    id,
                    state: DeviceState::Unknown,
                    device_type: device_config.device_type,
                    controller: device_config.controller.clone(),
                },
            )
        })
        .collect();

    Ok(Box::leak(Box::new(RwLock::new(devices))))
}
