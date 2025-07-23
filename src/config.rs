use std::path::Path;

use anyhow::Result;
use serde_derive::Deserialize;
use tokio::fs;

use crate::{
    automations::AutomationsConfig,
    devices::{DevicesConfig, controllers::ControllersConfig},
    server_backends::ServersConfig,
};

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    pub servers: ServersConfig,
    pub devices: DevicesConfig,
    pub controllers: ControllersConfig,
    pub automations: AutomationsConfig,
}

pub async fn load_config(path: impl AsRef<Path>) -> Result<Config> {
    Ok(serde_json::from_slice(&fs::read(path).await?)?)
}
