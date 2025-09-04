use anyhow::{Context, Result};
use chrono::{Days, Local, Utc};
use devicectrl_common::UpdateRequest;
use futures::future::try_join_all;
use serde_derive::Deserialize;
use sunrise::{Coordinates, DawnType, SolarDay, SolarEvent};
use tokio::time::sleep;

use crate::{AppState, devices::dispatch::process_update_request};

#[derive(Clone, Debug, Deserialize)]
pub struct SunsetLightTempAutomationConfig {
    coords: (f64, f64),
    updates: Vec<UpdateRequest>,
}

pub async fn start_automation(
    config: SunsetLightTempAutomationConfig,
    app_state: &AppState,
) -> Result<()> {
    let coord =
        Coordinates::new(config.coords.0, config.coords.1).context("invalid coordinates")?;
    loop {
        let sunset = SolarDay::new(coord, Local::now().naive_local().date())
            .event_time(SolarEvent::Dusk(DawnType::Civil));

        let now = Utc::now();
        let wait_time = if sunset > now {
            (sunset - now)
                .to_std()
                .context("sunset duration could not be found")?
        } else {
            let tmr_sunset = SolarDay::new(
                coord,
                Local::now()
                    .checked_add_days(Days::new(1))
                    .context("tmr could not be determined")?
                    .naive_local()
                    .date(),
            )
            .event_time(SolarEvent::Dusk(sunrise::DawnType::Civil));

            (tmr_sunset - now)
                .to_std()
                .context("tmr's sunset duration could not be found")?
        };

        log::debug!("waiting {wait_time:?} until sunset");
        sleep(wait_time).await;

        try_join_all(
            config
                .updates
                .iter()
                .map(|update| process_update_request(update, app_state)),
        )
        .await?;
    }
}
