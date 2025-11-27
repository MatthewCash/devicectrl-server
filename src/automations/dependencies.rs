use anyhow::{Context, Result};
use devicectrl_common::{
    DeviceId, DeviceState, UpdateRequest,
    device_types::switch::{SwitchPower, SwitchState},
    updates::AttributeUpdate,
};
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
                    && matches!(request.update, AttributeUpdate::Power(SwitchPower::On))
                {
                    log::debug!(
                        "updating {} as dependent of {}",
                        dependency.dependency_id,
                        dependency.dependent_id
                    );

                    process_update_request(
                        UpdateRequest {
                            device_id: dependency.dependency_id,
                            update: AttributeUpdate::Power(SwitchPower::On),
                        },
                        app_state,
                    )
                    .await
                    .context("failed to process dependent update request")?;
                }
            }
            // we can't detect state updates of individual attributes, so we have to restrict the check to only switch devices
            Hook::DeviceStateUpdate(update) => {
                if let Some(dependency) = config.iter().find(|d| d.dependent_id == update.device_id)
                    && matches!(
                        update.new_state,
                        DeviceState::Switch(SwitchState {
                            power: SwitchPower::On
                        })
                    )
                {
                    log::debug!(
                        "updating {} as dependent of {}",
                        dependency.dependency_id,
                        dependency.dependent_id
                    );

                    process_update_request(
                        UpdateRequest {
                            device_id: dependency.dependency_id,
                            update: AttributeUpdate::Power(SwitchPower::On),
                        },
                        app_state,
                    )
                    .await
                    .context("failed to process dependent update request")?;
                }

                if let Some(dependency) = config.iter().find(|d| d.dependent_id == update.device_id)
                    && matches!(
                        update.new_state,
                        DeviceState::Switch(SwitchState {
                            power: SwitchPower::Off
                        })
                    )
                {
                    log::debug!(
                        "updating {} as dependent of {}",
                        dependency.dependency_id,
                        dependency.dependent_id
                    );

                    process_update_request(
                        UpdateRequest {
                            device_id: dependency.dependency_id,
                            update: AttributeUpdate::Power(SwitchPower::Off),
                        },
                        app_state,
                    )
                    .await
                    .context("failed to process dependent state update")?;
                }
            }
            _ => {}
        }
    }
}
