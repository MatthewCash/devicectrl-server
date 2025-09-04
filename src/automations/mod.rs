use anyhow::Result;
use serde_derive::Deserialize;
use sunset_light_temp::SunsetLightTempAutomationConfig;
use tokio::task::JoinSet;

use crate::{AppState, automations::dependencies::DependencyAutomationConfig};

mod dependencies;
mod sunset_light_temp;

#[derive(Clone, Debug, Deserialize)]
pub struct AutomationsConfig {
    sunset_light_temp: Option<SunsetLightTempAutomationConfig>,
    dependencies: Option<DependencyAutomationConfig>,
}

macro_rules! spawn_automations {
    ( $tasks:expr, $config:expr, $app_state:expr, [$( $field:ident ),*] ) => {
        $(
            if let Some(ref config) = $config.$field {
                $tasks.spawn($field::start_automation(
                    config.clone(),
                    $app_state,
                ));
            }
        )*
    };
}

pub async fn start_automations(app_state: &'static AppState) -> Result<()> {
    let config = &app_state.config.automations;

    let mut tasks = JoinSet::new();

    spawn_automations!(tasks, config, app_state, [sunset_light_temp, dependencies]);

    while tasks.join_next().await.transpose()?.transpose()?.is_some() {}

    Ok(())
}
