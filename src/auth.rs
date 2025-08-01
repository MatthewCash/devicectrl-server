use anyhow::Result;
use p256::{ecdsa::SigningKey, pkcs8::DecodePrivateKey};
use serde::{Deserialize, de};

pub fn deserialize_signing_key<'de, D>(deserializer: D) -> Result<SigningKey, D::Error>
where
    D: de::Deserializer<'de>,
{
    let der_bytes = std::fs::read(String::deserialize(deserializer)?).map_err(de::Error::custom)?;
    SigningKey::from_pkcs8_der(&der_bytes).map_err(de::Error::custom)
}
