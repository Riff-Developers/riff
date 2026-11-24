use riff_api::models::Device;

use crate::remote::{CurrentDevice, PlaybackView};

#[derive(Clone, Debug, PartialEq)]
pub enum DeviceEntry {
    ThisDevice,
    Other(Device),
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceList {
    pub current: DeviceEntry,
    pub others: Vec<DeviceEntry>,
    pub online: bool,
}

impl DeviceList {
    pub fn from_view(view: &PlaybackView) -> Self {
        let (current, this_device) = match &view.current {
            CurrentDevice::ThisDevice => (DeviceEntry::ThisDevice, None),
            CurrentDevice::Other(device) => (
                DeviceEntry::Other(device.clone()),
                Some(DeviceEntry::ThisDevice),
            ),
        };
        let others = this_device
            .into_iter()
            .chain(view.others.iter().cloned().map(DeviceEntry::Other))
            .collect();
        Self {
            current,
            others,
            online: view.online,
        }
    }

    pub fn current_volume(&self) -> Option<u8> {
        match &self.current {
            DeviceEntry::Other(device) if device.supports_volume => {
                Some(device.volume_percent.unwrap_or(0))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use riff_api::models::DeviceKind;

    fn device(id: &str, volume: Option<u8>) -> Device {
        Device {
            id: id.to_string(),
            label: id.to_uppercase(),
            kind: DeviceKind::Speaker,
            volume_percent: volume,
            supports_volume: true,
        }
    }

    #[test]
    fn playing_here_lists_the_others() {
        let view = PlaybackView {
            online: true,
            others: vec![device("phone", None)],
            ..Default::default()
        };
        let list = DeviceList::from_view(&view);
        assert_eq!(list.current, DeviceEntry::ThisDevice);
        assert_eq!(list.others, [DeviceEntry::Other(device("phone", None))]);
        assert_eq!(list.current_volume(), None);
    }

    #[test]
    fn playing_elsewhere_offers_this_device() {
        let view = PlaybackView {
            online: true,
            current: CurrentDevice::Other(device("phone", Some(30))),
            others: vec![device("tv", None)],
            ..Default::default()
        };
        let list = DeviceList::from_view(&view);
        assert_eq!(list.current, DeviceEntry::Other(device("phone", Some(30))));
        assert_eq!(list.current_volume(), Some(30));
        assert_eq!(
            list.others,
            [
                DeviceEntry::ThisDevice,
                DeviceEntry::Other(device("tv", None))
            ]
        );
    }
}
