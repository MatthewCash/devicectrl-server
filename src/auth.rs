use std::collections::HashMap;

use anyhow::Result;
use devicectrl_common::{
    DeviceId,
    protocol::auth::{AuthKey, AuthPair},
};
use p256::{ecdsa::SigningKey, pkcs8::DecodePrivateKey};
use serde::{Deserialize, de};
use serde_derive::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct AuthConfig {
    pub allowed_keys: HashMap<DeviceId, AuthKey>,
}

pub async fn verify_auth(auth: &AuthPair, config: &AuthConfig) -> Result<bool> {
    Ok(config.allowed_keys.get(&auth.id) == Some(&auth.key))
}

pub fn deserialize_signing_key<'de, D>(deserializer: D) -> Result<SigningKey, D::Error>
where
    D: de::Deserializer<'de>,
{
    let der_bytes = std::fs::read(String::deserialize(deserializer)?).map_err(de::Error::custom)?;
    SigningKey::from_pkcs8_der(&der_bytes).map_err(de::Error::custom)
}
