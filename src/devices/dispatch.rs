use anyhow::{Context, Result};
use devicectrl_common::{UpdateNotification, UpdateRequest};
use futures::future::join_all;

use super::controllers::Controllers;
use crate::{AppState, hooks::Hook};

pub async fn process_update_request(request: UpdateRequest, app_state: &AppState) -> Result<()> {
    let devices = app_state.devices.read().await;

    let device = devices
        .get(&request.device_id)
        .context("Could not find device id!")?;

    app_state
        .hooks
        .sender
        .send(Hook::DeviceUpdateDispatch(request))
        .context("failed to send update request hook")?;

    Controllers::dispatch_update(app_state, device, &request).await
}

pub fn process_update_notification(update: UpdateNotification, app_state: &AppState) -> Result<()> {
    app_state
        .hooks
        .sender
        .send(Hook::DeviceStateUpdate(update))
        .context("failed to send update notification hook")?;

    Ok(())
}

pub async fn query_all_device_states(app_state: &AppState) {
    let devices = app_state.devices.read().await;

    join_all(devices.values().map(async |device| {
        if let Err(err) = Controllers::dispatch_query_state(app_state, device).await {
            log::error!("{:?}", err.context("Failed to dispatch state query"));
        }
    }))
    .await;
}
