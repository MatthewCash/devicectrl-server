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
pub struct SunsetDeviceConfig {
    device_id: DeviceId,
    color_temp_range: (u8, u8),
    total_duration_secs: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SunsetLightTempAutomationConfig {
    coords: (f64, f64),
    devices: Vec<SunsetDeviceConfig>,
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

        try_join_all(config.devices.iter().map(|update| async {
            for color_temp in update.color_temp_range.0..=update.color_temp_range.1 {
                let request = UpdateRequest {
                    device_id: update.device_id,
                    update: AttributeUpdate::ColorTemp(ColorTempUpdate {
                        light_color_temp: color_temp,
                    }),
                };
                process_update_request(request, app_state).await?;

                sleep(Duration::from_secs(
                    update.total_duration_secs
                        / (update.color_temp_range.1 - update.color_temp_range.0) as u64,
                ))
                .await;
            }

            Result::<()>::Ok(())
        }))
        .await?;
    }
}
