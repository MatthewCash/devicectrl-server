use anyhow::{Context, Result};
use serde_derive::Deserialize;

use crate::Devices;
use crate::devices::controllers::krypton::{
    KryptonController, KryptonControllerConfig, KryptonControllerGlobalConfig,
};
use crate::{AppState, devices::Device};
use devicectrl_common::UpdateRequest;
use govee::{GoveeController, GoveeControllerConfig, GoveeControllerGlobalConfig};
use simple::{SimpleController, SimpleControllerConfig, SimpleControllerGlobalConfig};
use tplink::{TplinkController, TplinkControllerConfig, TplinkControllerGlobalConfig};

pub mod govee;
pub mod krypton;
pub mod simple;
pub mod tplink;

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ControllerConfig {
    Simple(SimpleControllerConfig),
    Krypton(KryptonControllerConfig),
    Tplink(TplinkControllerConfig),
    Govee(GoveeControllerConfig),
}

#[derive(Clone, Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct ControllersConfig {
    Simple: Option<SimpleControllerGlobalConfig>,
    Krypton: Option<KryptonControllerGlobalConfig>,
    Tplink: Option<TplinkControllerGlobalConfig>,
    Govee: Option<GoveeControllerGlobalConfig>,
}

macro_rules! make_controllers {
    (
        $vis:vis struct $name:ident {
            $(
                $field:ident => $ctrl_ty:ident
            ),* $(,)?
        }
    ) => {
        #[allow(non_snake_case)]
        $vis struct $name {
            $(
                pub $field: Option<$ctrl_ty>,
            )*
        }

        impl $name {
            pub async fn new(config: &ControllersConfig) -> Result<Self> {
                Ok(Self {
                    $(
                    $field: if let Some(cfg) = &config.$field {
                        Some($ctrl_ty::new(cfg.clone()).await?)
                    } else {
                        None
                    },
                    )*
                })
            }

            pub fn start_listening(
                &'static self,
                devices: &'static Devices,
                app_state: &'static AppState,
            ) {
                $(
                    self.$field.as_ref().map(|c| {
                        let controller = c.clone();
                        tokio::spawn(async move {
                            if let Err(err) = controller.start_listening(devices, app_state).await {
                                log::error!("{} controller listener failed: {:?}", stringify!($field), err);
                            }
                        });
                    });
                )*
            }

            pub async fn dispatch_update(
                app_state: &AppState,
                device: &Device,
                request: &UpdateRequest,
            ) -> Result<()> {
                match &device.controller {
                    $(
                        ControllerConfig::$field(config) => {
                            app_state
                                .controllers
                                .$field
                                .as_ref()
                                .context(concat!(
                                    stringify!($field),
                                    " controller is not enabled!"
                                ))?
                                .send_update(config, device, request)
                                .await
                        }
                    ),*
                }
            }

            pub async fn dispatch_query_state(
                app_state: &AppState,
                device: &Device,
            ) -> Result<()> {
                match &device.controller {
                    $(
                        ControllerConfig::$field(config) => {
                            app_state
                                .controllers
                                .$field
                                .as_ref()
                                .context(concat!(
                                    stringify!($field),
                                    " controller is not enabled!"
                                ))?
                                .send_query(config, device)
                                .await
                        }
                    ),*
                }
            }
        }
    };
}

make_controllers! {
    pub struct Controllers {
        Simple => SimpleController,
        Krypton => KryptonController,
        Tplink => TplinkController,
        Govee => GoveeController,
    }
}
