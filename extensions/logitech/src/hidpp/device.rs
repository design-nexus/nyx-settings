//! Identifying a HID++ 2.0 device: protocol version, feature table, name, type
//! and firmware.

use super::link::{Error, Link, Result};

/// HID++ 2.0 feature ids this app knows about.
pub mod feature {
    pub const ROOT: u16 = 0x0000;
    pub const FEATURE_SET: u16 = 0x0001;
    pub const FIRMWARE: u16 = 0x0003;
    pub const NAME: u16 = 0x0005;
    pub const BATTERY: u16 = 0x1000;
    pub const BATTERY_VOLTAGE: u16 = 0x1001;
    pub const UNIFIED_BATTERY: u16 = 0x1004;
    pub const CHANGE_HOST: u16 = 0x1814;
    pub const HOSTS_INFO: u16 = 0x1815;
    pub const BACKLIGHT2: u16 = 0x1982;
    pub const REPROG_CONTROLS_V4: u16 = 0x1B04;
    pub const SMART_SHIFT: u16 = 0x2110;
    pub const SMART_SHIFT_ENHANCED: u16 = 0x2111;
    pub const HIRES_WHEEL: u16 = 0x2121;
    pub const THUMB_WHEEL: u16 = 0x2150;
    pub const ADJUSTABLE_DPI: u16 = 0x2201;
    pub const FN_INVERSION: u16 = 0x40A0;
    pub const NEW_FN_INVERSION: u16 = 0x40A2;
    pub const K375S_FN_INVERSION: u16 = 0x40A3;
    pub const DISABLE_KEYS: u16 = 0x4521;
    pub const REPORT_RATE: u16 = 0x8060;
    pub const EXTENDED_REPORT_RATE: u16 = 0x8061;
    pub const COLOR_LED_EFFECTS: u16 = 0x8070;
    pub const RGB_EFFECTS: u16 = 0x8071;
    pub const ONBOARD_PROFILES: u16 = 0x8100;
}

use feature as f;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    Keyboard,
    Mouse,
    Trackball,
    Touchpad,
    Numpad,
    Remote,
    Presenter,
    Headset,
    Receiver,
    #[default]
    Other,
}

impl Kind {
    /// From the DeviceName feature's device type.
    pub fn from_device_type(t: u8) -> Self {
        match t {
            0 => Kind::Keyboard,
            1 => Kind::Remote,
            2 => Kind::Numpad,
            3 => Kind::Mouse,
            4 => Kind::Touchpad,
            5 => Kind::Trackball,
            6 => Kind::Presenter,
            7 => Kind::Receiver,
            8 => Kind::Headset,
            _ => Kind::Other,
        }
    }

    /// From the low nibble of a receiver's pairing information.
    pub fn from_receiver_code(code: u8) -> Self {
        match code & 0x0F {
            1 => Kind::Keyboard,
            2 => Kind::Mouse,
            3 => Kind::Numpad,
            4 => Kind::Presenter,
            7 => Kind::Remote,
            8 => Kind::Trackball,
            9 => Kind::Touchpad,
            _ => Kind::Other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Keyboard => "Keyboard",
            Kind::Mouse => "Mouse",
            Kind::Trackball => "Trackball",
            Kind::Touchpad => "Touchpad",
            Kind::Numpad => "Number pad",
            Kind::Remote => "Remote",
            Kind::Presenter => "Presenter",
            Kind::Headset => "Headset",
            Kind::Receiver => "Receiver",
            Kind::Other => "Device",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Kind::Keyboard | Kind::Numpad => "input-keyboard-symbolic",
            Kind::Mouse | Kind::Trackball => "input-mouse-symbolic",
            Kind::Touchpad => "input-touchpad-symbolic",
            Kind::Headset => "audio-headset-symbolic",
            Kind::Receiver => "network-wireless-symbolic",
            Kind::Remote | Kind::Presenter | Kind::Other => "input-gaming-symbolic",
        }
    }
}

/// What a device is and which features it has.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Info {
    /// Device index: a receiver slot (1–6), or `0xFF` when connected directly.
    pub index: u8,
    pub name: String,
    pub kind: Kind,
    pub protocol: (u8, u8),
    /// (feature id, feature index)
    pub features: Vec<(u16, u8)>,
    pub firmware: Vec<String>,
    /// Wireless product id (or USB product id for direct devices).
    pub wpid: u16,
    pub serial: String,
}

impl Info {
    pub fn index_of(&self, id: u16) -> Option<u8> {
        self.features.iter().find(|(fid, _)| *fid == id).map(|(_, i)| *i)
    }

    pub fn has(&self, id: u16) -> bool {
        self.index_of(id).is_some()
    }

    /// The first of `ids` this device has.
    pub fn first_of(&self, ids: &[u16]) -> Option<u16> {
        ids.iter().copied().find(|id| self.has(*id))
    }

    /// Call function `function` of feature `id`.
    pub fn call(&self, link: &mut Link, id: u16, function: u8, params: &[u8]) -> Result<Vec<u8>> {
        let index = self.index_of(id).ok_or(Error::NoFeature(id))?;
        link.call(self.index, index, function, params)
    }

    /// A stable key for saved settings: model plus serial when we have one.
    pub fn key(&self) -> String {
        let model = if self.wpid != 0 { format!("{:04x}", self.wpid) } else { slug(&self.name) };
        if self.serial.is_empty() { model } else { format!("{model}-{}", self.serial.to_lowercase()) }
    }

    pub fn is_gaming(&self) -> bool {
        self.has(f::ONBOARD_PROFILES) || self.has(f::COLOR_LED_EFFECTS) || self.has(f::RGB_EFFECTS)
    }
}

fn slug(name: &str) -> String {
    name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

/// Reply bytes, or `Error::Short` if the reply is too short.
pub fn need(r: &[u8], len: usize) -> Result<&[u8]> {
    if r.len() >= len { Ok(r) } else { Err(Error::Short) }
}

pub fn be16(r: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([r[at], r[at + 1]])
}

/// Ping the device. `Ok(None)` means it only speaks HID++ 1.0.
pub fn ping(link: &mut Link, index: u8) -> Result<Option<(u8, u8)>> {
    match link.call(index, 0x00, 1, &[0, 0, 0x5A]) {
        Ok(r) => {
            let r = need(&r, 2)?;
            Ok(Some((r[0], r[1])))
        }
        Err(Error::V10(0x01)) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Look up a feature's index through the root feature (0 if absent).
pub fn feature_index(link: &mut Link, index: u8, id: u16) -> Result<Option<u8>> {
    let r = link.call(index, 0x00, 0, &id.to_be_bytes())?;
    let r = need(&r, 1)?;
    Ok((r[0] != 0).then_some(r[0]))
}

/// Read everything that identifies the device. `wpid`, `serial`, `name` and
/// `kind` are what the receiver already told us, used when the device can't
/// report them itself.
pub fn probe(link: &mut Link, index: u8, wpid: u16, serial: &str, name: &str, kind: Kind) -> Result<Info> {
    let mut info =
        Info { index, name: name.to_string(), kind, wpid, serial: serial.to_string(), protocol: (1, 0), ..Default::default() };
    let Some(protocol) = ping(link, index)? else { return Ok(info) };
    info.protocol = protocol;
    info.features.push((f::ROOT, 0));

    if let Some(fs) = feature_index(link, index, f::FEATURE_SET)? {
        let count = need(&link.call(index, fs, 0, &[])?, 1)?[0];
        for i in 1..=count {
            let r = link.call(index, fs, 1, &[i])?;
            let r = need(&r, 2)?;
            info.features.push((be16(r, 0), i));
        }
    }

    if info.has(f::NAME) {
        let len = need(&info.call(link, f::NAME, 0, &[])?, 1)?[0] as usize;
        let mut bytes = Vec::new();
        while bytes.len() < len {
            let chunk = info.call(link, f::NAME, 1, &[bytes.len() as u8])?;
            let take = (len - bytes.len()).min(chunk.len());
            if take == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..take]);
        }
        let n = String::from_utf8_lossy(&bytes).trim_matches(char::from(0)).trim().to_string();
        if !n.is_empty() {
            info.name = n;
        }
        if let Ok(r) = info.call(link, f::NAME, 2, &[])
            && let Some(t) = r.first()
        {
            info.kind = Kind::from_device_type(*t);
        }
    }

    if info.has(f::FIRMWARE) {
        let r = info.call(link, f::FIRMWARE, 0, &[])?;
        let r = need(&r, 5)?;
        let entities = r[0];
        if info.serial.is_empty() {
            let unit = &r[1..5];
            if unit.iter().any(|b| *b != 0) {
                info.serial = unit.iter().map(|b| format!("{b:02X}")).collect();
            }
        }
        for e in 0..entities {
            if let Ok(r) = info.call(link, f::FIRMWARE, 1, &[e])
                && r.len() >= 8
                && let Some(fw) = format_firmware(&r)
            {
                info.firmware.push(fw);
            }
        }
    }
    Ok(info)
}

fn format_firmware(r: &[u8]) -> Option<String> {
    let kind = match r[0] & 0x0F {
        0 => "Firmware",
        1 => "Bootloader",
        2 => "Hardware",
        3 => "Touchpad",
        4 => "Sensor",
        _ => return None,
    };
    let prefix: String = r[1..4].iter().filter(|b| b.is_ascii_graphic()).map(|b| *b as char).collect();
    let build = be16(r, 6);
    Some(if kind == "Hardware" {
        format!("{kind} {}", r[4])
    } else {
        format!("{kind} {prefix} {:02X}.{:02X}.B{build:04X}", r[4], r[5]).replace("  ", " ")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn firmware_strings() {
        assert_eq!(
            format_firmware(&[0, b'R', b'B', b'M', 0x16, 0x00, 0x00, 0x16, 0, 0]).as_deref(),
            Some("Firmware RBM 16.00.B0016")
        );
        assert_eq!(format_firmware(&[2, 0, 0, 0, 72, 0, 0, 0]).as_deref(), Some("Hardware 72"));
    }

    #[test]
    fn keys_are_stable() {
        let i = Info { wpid: 0xB034, serial: "ABCD1234".into(), ..Default::default() };
        assert_eq!(i.key(), "b034-abcd1234");
        let j = Info { name: "MX Keys".into(), ..Default::default() };
        assert_eq!(j.key(), "mx-keys");
    }
}
