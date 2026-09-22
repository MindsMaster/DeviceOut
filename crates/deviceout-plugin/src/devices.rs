use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use parking_lot::{RwLock, RwLockReadGuard};

use deviceout_sink::wasapi::{list_output_devices, ComGuard};
use deviceout_sink::DeviceInfo;

use crate::spawn;

#[derive(Default)]
pub struct Devices {
    list: RwLock<Vec<DeviceInfo>>,
    scanning: Arc<AtomicBool>,
}

impl Devices {
    pub(crate) fn read(&self) -> RwLockReadGuard<'_, Vec<DeviceInfo>> {
        self.list.read()
    }

    pub(crate) fn scan(&self) {
        let found = enumerate();
        *self.list.write() = found;
    }

    pub(crate) fn scan_async(self: &Arc<Self>) {
        let devices = Arc::clone(self);
        let scanning = Arc::clone(&self.scanning);
        spawn::detach_once("deviceout-devices", &scanning, move || devices.scan());
    }
}

impl From<Vec<DeviceInfo>> for Devices {
    fn from(list: Vec<DeviceInfo>) -> Self {
        Self {
            list: RwLock::new(list),
            scanning: Arc::default(),
        }
    }
}

fn enumerate() -> Vec<DeviceInfo> {
    let Ok(_com) = ComGuard::new() else {
        return Vec::new();
    };
    list_output_devices().unwrap_or_default()
}
