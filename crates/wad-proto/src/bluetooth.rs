//! Bluetooth (phase 8, Settings ▸ Bluetooth): the adapter, the devices
//! paired and in range, and what pairing asks.
//!
//! On wadd's socket (`api_phase8.rs`'s `Bluetooth` trait), each answering
//! with [`Bluetooth`] as it is after (also the `bluetooth` event):
//!
//! | Route | What |
//! |---|---|
//! | `GET /v1/bluetooth` | [`Bluetooth`] |
//! | `POST /v1/bluetooth/power` | on or off ([`crate::network::Switch`]) |
//! | `POST /v1/bluetooth/scan` | looking for devices, or not ([`crate::network::Switch`]) |
//! | `POST /v1/bluetooth/devices/{address}/pair` | starts pairing: it may ask ([`BtPrompt`]); paired, it connects |
//! | `POST /v1/bluetooth/devices/{address}/connect` | connects a paired one |
//! | `POST /v1/bluetooth/devices/{address}/disconnect` | |
//! | `POST /v1/bluetooth/devices/{address}/forget` | unpairs it |
//! | `POST /v1/bluetooth/prompt/{id}` | answers what pairing asked ([`BtAnswer`]) |
//!
//! Pairing doesn't wait: the call answers at once, and what it asks comes
//! as the `btPrompt` event (none once it's answered, cancelled or timed
//! out); paired, the device shows so in the `bluetooth` event. It failing
//! is a notice. A device unknown: 404; Bluetooth off: 409.

use serde::{Deserialize, Serialize};

/// `GET /v1/bluetooth` (the `bluetooth` event).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bluetooth {
    pub on: bool,
    /// What other devices see this machine as.
    #[serde(default)]
    pub discoverable_name: String,
    /// Paired ones first (connected first), then the ones in range (nameless ones left out).
    #[serde(default)]
    pub devices: Vec<BtDevice>,
    /// Looking for devices now.
    #[serde(default)]
    pub scanning: bool,
}

/// A device: paired, or in range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BtDevice {
    /// Its address (`AA:BB:CC:DD:EE:FF`): what the routes know it by.
    pub address: String,
    pub name: String,
    pub kind: BtKind,
    pub paired: bool,
    pub connected: bool,
    /// Its battery, 0-100, when it says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery: Option<u8>,
}

/// What a device is (from its class: BlueZ's icon), the spike's kinds and `other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BtKind {
    Headphones,
    Speaker,
    Mouse,
    Keyboard,
    Phone,
    Controller,
    Other,
}

/// What pairing asks (the `btPrompt` event): the shell shows it as a dialog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BtPrompt {
    /// What its answer is posted to (`POST /v1/bluetooth/prompt/{id}`).
    pub id: String,
    /// The device's address.
    pub device: String,
    /// Its name, to say which.
    pub name: String,
    #[serde(flatten)]
    pub kind: BtAsk,
}

/// What it asks (`kind` beside the prompt's fields).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BtAsk {
    /// Does the device show `code`? Yes or no.
    Confirm { code: String },
    /// Type `code` on the device, then Enter (a keyboard); nothing to answer but No.
    Show { code: String },
    /// Enter the PIN the device wants (an old one: often 0000): [`BtAnswer::pin`].
    Enter,
    /// Let it connect? Yes or no.
    Authorize,
}

/// `POST /v1/bluetooth/prompt/{id}`: yes or no, and an `Enter` prompt's PIN.
/// The prompt gone already: 404.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BtAnswer {
    pub accept: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_prompt_says_what_it_asks_beside_its_fields() {
        let p = BtPrompt {
            id: "bt-1".into(),
            device: "AA:BB:CC:DD:EE:01".into(),
            name: "Pixel Buds".into(),
            kind: BtAsk::Confirm { code: "123456".into() },
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!((v["kind"].as_str(), v["code"].as_str()), (Some("confirm"), Some("123456")));
        assert_eq!(serde_json::from_value::<BtPrompt>(v).unwrap(), p);
        let enter: BtPrompt =
            serde_json::from_value(json!({"id": "bt-2", "device": "AA", "name": "Old", "kind": "enter"})).unwrap();
        assert_eq!(enter.kind, BtAsk::Enter);
        let a: BtAnswer = serde_json::from_value(json!({"accept": true})).unwrap();
        assert_eq!(a.pin, None);
        let b: Bluetooth = serde_json::from_value(json!({"on": false})).unwrap();
        assert_eq!(b, Bluetooth::default());
    }
}
