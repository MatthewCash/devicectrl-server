use anyhow::Result;
use p256::{
    ecdsa::{SigningKey, VerifyingKey},
    pkcs8::{DecodePrivateKey, DecodePublicKey},
};
use serde::{Deserialize, de};
use serde_derive::Deserialize;
use std::path::Path;
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

pub fn deserialize_signing_key<'de, D>(deserializer: D) -> Result<SigningKey, D::Error>
where
    D: de::Deserializer<'de>,
{
    let der_bytes = std::fs::read(String::deserialize(deserializer)?).map_err(de::Error::custom)?;
    SigningKey::from_pkcs8_der(&der_bytes).map_err(de::Error::custom)
}

pub fn deserialize_verifying_key<'de, D>(deserializer: D) -> Result<VerifyingKey, D::Error>
where
    D: de::Deserializer<'de>,
{
    let der_bytes = std::fs::read(String::deserialize(deserializer)?).map_err(de::Error::custom)?;
    VerifyingKey::from_public_key_der(&der_bytes).map_err(de::Error::custom)
}

pub fn deserialize_file_path_bytes<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: de::Deserializer<'de>,
{
    std::fs::read(String::deserialize(deserializer)?).map_err(de::Error::custom)
}
