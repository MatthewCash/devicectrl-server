use anyhow::{Context, Result};
use devicectrl_common::{SceneId, UpdateRequest};
use futures::future::try_join_all;
use std::collections::HashMap;

use crate::{AppState, devices::dispatch::process_update_request, hooks::Hook};

pub type ScenesConfig = HashMap<SceneId, Vec<UpdateRequest>>;

pub async fn process_scene_activate(request: &SceneId, app_state: &AppState) -> Result<()> {
    let scene = app_state
        .config
        .scenes
        .get(request)
        .context("Could not find scene!")?;

    app_state
        .hooks
        .sender
        .send(Hook::SceneActivate(*request))
        .context("failed to send scene activation hook")?;

    try_join_all(
        scene
            .iter()
            .map(|update| process_update_request(*update, app_state)),
    )
    .await
    .context("failed to update activate scene")?;

    Ok(())
}
