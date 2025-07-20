use devicectrl_common::{UpdateNotification, UpdateRequest};
use tokio::sync::broadcast;

#[derive(Clone, Debug)]
pub enum Hook {
    DeviceUpdateDispatch(UpdateRequest),
    DeviceStateUpdate(UpdateNotification),
}

pub struct HooksChannel {
    pub sender: broadcast::Sender<Hook>,
    pub receiver: broadcast::Receiver<Hook>,
}

impl Default for HooksChannel {
    fn default() -> Self {
        let (sender, receiver) = broadcast::channel(16);
        Self { sender, receiver }
    }
}
