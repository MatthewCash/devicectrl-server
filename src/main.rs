use anyhow::{Context, Result};
use automations::start_automations;
use devices::{Devices, controllers::Controllers, dispatch::query_all_device_states, load_devices};
use sd_notify::NotifyState;
use std::{env, path::PathBuf};
use tokio::task::JoinSet;
use tracing_subscriber::{EnvFilter, filter::LevelFilter};

use config::Config;
use hooks::HooksChannel;
use server_backends::start_servers;

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

    let state = Box::leak(Box::new(AppState {
        devices: load_devices(&config).await?,
        controllers: Controllers::new(&config.controllers).await?,
        hooks: Default::default(),
        config,
    }));

    state
        .controllers
        .start_listening(state.devices.clone(), state);

    tokio::spawn(query_all_device_states(state));

    let mut tasks = JoinSet::new();

    tasks.spawn(start_servers(state));

    tasks.spawn(start_automations(state));

    let _ = sd_notify::notify(false, &[NotifyState::Ready]);

    while tasks.join_next().await.transpose()?.transpose()?.is_some() {}

    Ok(())
}
