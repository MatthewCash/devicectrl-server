use anyhow::{Context, Result};
use devicectrl_common::device_types::switch::SwitchState;
use devicectrl_common::{DeviceId, UpdateRequest, device_types::switch::SwitchStateUpdate};
use devicectrl_common::{DeviceState, DeviceStateUpdate};
use serde_derive::Deserialize;

use crate::{AppState, devices::dispatch::process_update_request, hooks::Hook};

#[derive(Clone, Debug, Deserialize)]
pub struct DependencyRelationship {
    dependent_id: DeviceId,
    dependency_id: DeviceId,
}

pub type DependencyAutomationConfig = Vec<DependencyRelationship>;

pub async fn start_automation(
    config: DependencyAutomationConfig,
    app_state: &AppState,
) -> Result<()> {
    let mut receiver = app_state.hooks.receiver.resubscribe();

    loop {
        let event = receiver.recv().await?;

        match event {
            // dependencies must be turned on as soon as their dependent is
            Hook::DeviceUpdateDispatch(request) => {
                if let Some(dependency) =
                    config.iter().find(|d| d.dependent_id == request.device_id)
                    && matches!(
                        request.change_to,
                        DeviceStateUpdate::Switch(SwitchStateUpdate { power: Some(true) })
                    )
                {
                    log::debug!(
                        "updating {} as dependent of {}",
                        dependency.dependency_id,
                        dependency.dependent_id
                    );

                    process_update_request(
                        &UpdateRequest {
                            device_id: dependency.dependency_id,
                            change_to: DeviceStateUpdate::Switch(SwitchStateUpdate {
                                power: Some(true),
                            }),
                        },
                        app_state,
                    )
                    .await
                    .context("failed to process dependent update request")?;
                }
            }
            Hook::DeviceStateUpdate(update) => {
                if let Some(dependency) = config.iter().find(|d| d.dependent_id == update.device_id)
                    && matches!(
                        update.new_state,
                        DeviceState::Switch(SwitchState { power: true })
                    )
                {
                    log::debug!(
                        "updating {} as dependent of {}",
                        dependency.dependency_id,
                        dependency.dependent_id
                    );

                    process_update_request(
                        &UpdateRequest {
                            device_id: dependency.dependency_id,
                            change_to: DeviceStateUpdate::Switch(SwitchStateUpdate {
                                power: Some(true),
                            }),
                        },
                        app_state,
                    )
                    .await
                    .context("failed to process dependent update request")?;
                }

                if let Some(dependency) = config.iter().find(|d| d.dependent_id == update.device_id)
                    && matches!(
                        update.new_state,
                        DeviceState::Switch(SwitchState { power: false })
                    )
                {
                    log::debug!(
                        "updating {} as dependent of {}",
                        dependency.dependency_id,
                        dependency.dependent_id
                    );

                    process_update_request(
                        &UpdateRequest {
                            device_id: dependency.dependency_id,
                            change_to: DeviceStateUpdate::Switch(SwitchStateUpdate {
                                power: Some(false),
                            }),
                        },
                        app_state,
                    )
                    .await
                    .context("failed to process dependent state update")?;
                }
            }
        }
    }
}
