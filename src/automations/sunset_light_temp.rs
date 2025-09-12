use anyhow::{Context, Result};
use chrono::{Days, Local, Utc};
use devicectrl_common::{
    DeviceId, UpdateRequest,
    updates::{AttributeUpdate, ColorTempUpdate},
};
use futures::future::try_join_all;
use serde_derive::Deserialize;
use std::time::Duration;
use sunrise::{Coordinates, SolarDay, SolarEvent};
use tokio::time::sleep;

use crate::{AppState, devices::dispatch::process_update_request};

#[derive(Clone, Debug, Deserialize)]
pub struct SunsetLightTempAutomationConfig {
    coords: (f64, f64),
    device_ids: Vec<DeviceId>,
}

pub async fn start_automation(
    config: SunsetLightTempAutomationConfig,
    app_state: &AppState,
) -> Result<()> {
    let coord =
        Coordinates::new(config.coords.0, config.coords.1).context("invalid coordinates")?;
    loop {
        let sunset =
            SolarDay::new(coord, Local::now().naive_local().date()).event_time(SolarEvent::Sunset);

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

        // process takes a bit less than 22 mins
        for light_color_temp in 0..255 {
            let updates = config.device_ids.iter().map(|id| UpdateRequest {
                device_id: *id,
                update: AttributeUpdate::ColorTemp(ColorTempUpdate { light_color_temp }),
            });

            try_join_all(updates.map(|update| process_update_request(update, app_state))).await?;

            sleep(Duration::from_secs(5)).await;
        }
    }
}
