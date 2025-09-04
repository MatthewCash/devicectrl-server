use std::{env, path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use automations::start_automations;
use devices::{Devices, controllers::Controllers, dispatch::query_all_device_states, load_devices};
use sd_notify::NotifyState;
use tokio::task::JoinSet;
use tracing_subscriber::{EnvFilter, filter::LevelFilter};

use config::Config;
use hooks::HooksChannel;
use server_backends::{http, tcp, websocket};

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

    if let Some(ref tcp_config) = state.config.servers.tcp {
        tasks.spawn(tcp::start_listening(tcp_config.clone(), state.clone()));
    }

    if let Some(ref http_config) = state.config.servers.http {
        tasks.spawn(http::start_listening(http_config.clone(), state.clone()));
    }

    if let Some(ref websocket_config) = state.config.servers.websocket {
        tasks.spawn(websocket::start_listening(
            websocket_config.clone(),
            state.clone(),
        ));
    }

    tasks.spawn(start_automations(state));

    let _ = sd_notify::notify(false, &[NotifyState::Ready]);

    while tasks.join_next().await.transpose()?.transpose()?.is_some() {}

    Ok(())
}
