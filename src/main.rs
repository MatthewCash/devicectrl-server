use std::{env, path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use automations::start_automations;
use config::Config;
use devices::{Devices, controllers::Controllers, dispatch::query_all_device_states, load_devices};
use sd_notify::NotifyState;
use server_backends::tcp;
use tokio::task::JoinSet;
use tracing_subscriber::{EnvFilter, filter::LevelFilter};

use crate::hooks::HooksChannel;

mod auth;
mod automations;
mod config;
mod devices;
mod hooks;
mod server_backends;

pub struct AppState {
    pub config: Config,
    pub devices: Devices,
    pub hooks: HooksChannel,
    pub controllers: Controllers,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .without_time() // systemd logs already include timestamps
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .with_env_var("LOG_LEVEL")
                .from_env()?,
        )
        .init();

    let config = config::load_config(&PathBuf::from(
        env::var("CONFIG_PATH").expect("CONFIG_PATH env var missing!"),
    ))
    .await
    .context("failed to load config")?;

    let state = Arc::new(AppState {
        devices: load_devices(&config).await?,
        controllers: Controllers::new(&config.controllers).await?,
        hooks: Default::default(),
        config,
    });

    state
        .controllers
        .start_listening(state.devices.clone(), state.clone());

    tokio::spawn(query_all_device_states(state.clone()));

    let mut tasks = JoinSet::new();

    if let Some(ref tcp_config) = state.config.servers.tcp {
        tasks.spawn(tcp::start_listening(tcp_config.clone(), state.clone()));
    }

    tasks.spawn(start_automations(state.clone()));

    let _ = sd_notify::notify(false, &[NotifyState::Ready]);

    while tasks.join_next().await.transpose()?.transpose()?.is_some() {}

    Ok(())
}
