//! Simulated receivers and devices that answer HID++ like the real thing.
//! Used by the tests and by `LOGI_MOCK=mx-master-3s,mx-keys,g502x,mx-anywhere-3s`
//! to run the whole app without hardware (add `denied` to pretend access isn't allowed,
//! `gearbench` to pretend Gearbench is set up).

use super::device::feature as f;
use super::hidraw::{BUS_BLUETOOTH, BUS_USB, NodeInfo};
use super::link::{GET_LONG_REGISTER, Io, SET_LONG_REGISTER, SET_REGISTER};
use super::receiver::ReceiverKind;
use std::collections::VecDeque;
use std::io;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct SimDevice {
    name: &'static str,
    device_type: u8,
    receiver_code: u8,
    wpid: u16,
    serial: [u8; 4],
    features: Vec<u16>,
    online: bool,
    dpi: u16,
    dpi_list: Vec<u16>,
    rate_mask: u16,
    rate: u8,
    wheel: (u8, u8),
    hires: u8,
    thumb: u8,
    fn_swap: u8,
    disable_caps: u8,
    disabled: u8,
    hosts: (u8, u8),
    backlight: [u8; 12],
    profiles: u8,
    onboard_mode: u8,
    profile: u8,
    zones: Vec<(u16, u8, [u8; 10])>,
    controls: Vec<(u16, u8, u8, u8, u16)>,
    battery: (u8, u8),
    /// Main firmware: prefix, major, minor, build.
    firmware: ([u8; 3], u8, u8, u16),
    /// Easy-Switch host names; empty means not paired.
    host_names: Vec<&'static str>,
}

impl SimDevice {
    fn base(name: &'static str, device_type: u8, receiver_code: u8, wpid: u16, serial: [u8; 4]) -> Self {
        Self {
            name,
            device_type,
            receiver_code,
            wpid,
            serial,
            features: vec![f::FEATURE_SET, f::FIRMWARE, f::NAME, f::UNIFIED_BATTERY],
            online: true,
            dpi: 1000,
            dpi_list: vec![200, 0xE000 | 50, 8000],
            rate_mask: 0,
            rate: 3,
            wheel: (2, 10),
            hires: 0x02,
            thumb: 0,
            fn_swap: 1,
            disable_caps: 0,
            disabled: 0,
            hosts: (3, 0),
            backlight: [1, 0x08, 0x1F, 0, 0, 4, 0, 12, 0, 60, 0, 60],
            profiles: 0,
            onboard_mode: 2,
            profile: 1,
            zones: vec![],
            controls: vec![],
            battery: (80, 0),
            firmware: (*b"RBM", 0x21, 0x02, 0x0014),
            host_names: vec![],
        }
    }

    pub fn model(name: &str) -> Option<Self> {
        Some(match name {
            "mx-master-3s" => {
                let mut d = Self::base("MX Master 3S", 3, 2, 0xB034, [0x8A, 0x21, 0x5C, 0x03]);
                d.features.extend([
                    f::CHANGE_HOST,
                    f::REPROG_CONTROLS_V4,
                    f::SMART_SHIFT_ENHANCED,
                    f::HIRES_WHEEL,
                    f::THUMB_WHEEL,
                    f::ADJUSTABLE_DPI,
                    f::EXTENDED_REPORT_RATE,
                    f::HOSTS_INFO,
                ]);
                d.rate_mask = 0x0F;
                d.wheel = (2, 12);
                d.battery = (70, 0);
                d.firmware = (*b"RBM", 0x12, 0x02, 0x0014);
                d.host_names = vec!["Hades", "", ""];
                d.controls = vec![
                    (0x50, 0x01, 1, 0x00, 0x50),
                    (0x51, 0x01, 1, 0x00, 0x51),
                    (0x52, 0x31, 2, 0x03, 0x52),
                    (0x53, 0x31, 2, 0x03, 0x53),
                    (0x56, 0x31, 2, 0x03, 0x56),
                    (0xC3, 0x31, 2, 0x03, 0x52),
                    (0xC4, 0x31, 2, 0x03, 0xC4),
                ];
                d
            }
            "mx-keys" => {
                let mut d = Self::base("MX Keys", 0, 1, 0xB35B, [0x8F, 0x2A, 0x1C, 0x04]);
                d.features.extend([
                    f::CHANGE_HOST,
                    f::BACKLIGHT2,
                    f::REPROG_CONTROLS_V4,
                    f::K375S_FN_INVERSION,
                    f::DISABLE_KEYS,
                    f::HOSTS_INFO,
                ]);
                d.disable_caps = 0x19;
                d.disabled = 0x01;
                d.battery = (35, 0);
                d.firmware = (*b"RBK", 0x04, 0x01, 0x0012);
                d.host_names = vec!["Hades", "", ""];
                d.controls = [0xC7, 0xC8, 0x0111, 0x0112, 0xE0, 0xD4, 0x0103, 0xE4, 0x0114, 0x0116, 0x0117, 0x0118]
                    .into_iter()
                    .map(|cid| (cid, 0x11, 1, 0x01, cid))
                    .collect();
                d
            }
            "g502x" => {
                let mut d = Self::base("G502 X LIGHTSPEED", 3, 2, 0x409F, [0x55, 0x10, 0xA2, 0x9E]);
                d.features.extend([
                    f::ADJUSTABLE_DPI,
                    f::EXTENDED_REPORT_RATE,
                    f::ONBOARD_PROFILES,
                    f::COLOR_LED_EFFECTS,
                    f::REPROG_CONTROLS_V4,
                ]);
                d.dpi = 1600;
                d.dpi_list = vec![100, 0xE000 | 50, 25600];
                d.rate_mask = 0x0F;
                d.profiles = 5;
                d.onboard_mode = 1;
                d.profile = 2;
                d.battery = (12, 0);
                d.firmware = (*b"MPM", 0x54, 0x03, 0x0007);
                d.zones = vec![(1, 3, [0xA8, 0x55, 0xF7, 0x0B, 0xB8, 0, 100, 0, 0, 0]), (2, 1, [0xA8, 0x55, 0xF7, 0, 0, 0, 0, 0, 0, 0])];
                d.controls = vec![
                    (0x50, 0x01, 1, 0x00, 0x50),
                    (0x51, 0x01, 1, 0x00, 0x51),
                    (0x52, 0x31, 2, 0x03, 0x52),
                    (0x53, 0x31, 2, 0x03, 0x53),
                    (0x56, 0x31, 2, 0x03, 0x56),
                    (0xFD, 0x31, 2, 0x03, 0xFD),
                ];
                d
            }
            "mx-anywhere-3s" => {
                let mut d = Self::base("MX Anywhere 3S", 3, 2, 0xB037, [0x3C, 0x00, 0x19, 0xD1]);
                d.features.extend([f::CHANGE_HOST, f::SMART_SHIFT, f::HIRES_WHEEL, f::ADJUSTABLE_DPI]);
                d.hosts = (3, 1);
                d.battery = (100, 3);
                d
            }
            _ => return None,
        })
    }

    fn handle(&mut self, index: u8, func: u8, p: &[u8]) -> Result<Vec<u8>, u8> {
        let p = |i: usize| p.get(i).copied().unwrap_or(0);
        let be = |v: u16| v.to_be_bytes();
        if index == 0 {
            return match func {
                0 => {
                    let id = u16::from_be_bytes([p(0), p(1)]);
                    let i = if id == 0 { 0 } else { self.features.iter().position(|x| *x == id).map(|i| i + 1).unwrap_or(0) };
                    Ok(vec![i as u8, 0, 0])
                }
                1 => Ok(vec![4, 5, p(2)]),
                _ => Err(0x07),
            };
        }
        let Some(&feature) = self.features.get(index as usize - 1) else { return Err(0x06) };
        match (feature, func) {
            (f::FEATURE_SET, 0) => Ok(vec![self.features.len() as u8]),
            (f::FEATURE_SET, 1) => {
                let id = self.features.get(p(0) as usize - 1).copied().ok_or(0x02u8)?;
                Ok(be(id).to_vec())
            }
            (f::FIRMWARE, 0) => Ok(vec![2, self.serial[0], self.serial[1], self.serial[2], self.serial[3]]),
            (f::FIRMWARE, 1) => Ok(match p(0) {
                0 => {
                    let (prefix, major, minor, build) = self.firmware;
                    let b = be(build);
                    vec![0, prefix[0], prefix[1], prefix[2], major, minor, b[0], b[1]]
                }
                _ => vec![1, b'B', b'L', b'2', 0x02, 0x00, 0x00, 0x05],
            }),
            (f::NAME, 0) => Ok(vec![self.name.len() as u8]),
            (f::NAME, 1) => Ok(self.name.as_bytes().iter().skip(p(0) as usize).take(16).copied().collect()),
            (f::NAME, 2) => Ok(vec![self.device_type]),
            (f::UNIFIED_BATTERY, 1) => {
                let level = if self.battery.0 > 60 { 0x08 } else if self.battery.0 > 20 { 0x04 } else { 0x02 };
                Ok(vec![self.battery.0, level, self.battery.1, 0])
            }
            (f::ADJUSTABLE_DPI, 1) => {
                let mut r = vec![0];
                for w in &self.dpi_list {
                    r.extend(be(*w));
                }
                r.extend([0, 0]);
                Ok(r)
            }
            (f::ADJUSTABLE_DPI, 2) => {
                let mut r = vec![0];
                r.extend(be(self.dpi));
                r.extend(be(1000));
                Ok(r)
            }
            (f::ADJUSTABLE_DPI, 3) => {
                self.dpi = u16::from_be_bytes([p(1), p(2)]);
                Ok(vec![0])
            }
            (f::EXTENDED_REPORT_RATE, 1) => Ok(be(self.rate_mask).to_vec()),
            (f::EXTENDED_REPORT_RATE, 2) => Ok(vec![self.rate]),
            (f::EXTENDED_REPORT_RATE, 3) => {
                self.rate = p(0);
                Ok(vec![])
            }
            (f::SMART_SHIFT, 0) | (f::SMART_SHIFT_ENHANCED, 1) => Ok(vec![self.wheel.0, self.wheel.1, 0]),
            (f::SMART_SHIFT, 1) | (f::SMART_SHIFT_ENHANCED, 2) => {
                if p(0) != 0 {
                    self.wheel.0 = p(0);
                }
                if p(1) != 0 {
                    self.wheel.1 = p(1);
                }
                Ok(vec![])
            }
            (f::HIRES_WHEEL, 0) => Ok(vec![8, 0x0C]),
            (f::HIRES_WHEEL, 1) => Ok(vec![self.hires]),
            (f::HIRES_WHEEL, 2) => {
                self.hires = p(0);
                Ok(vec![self.hires])
            }
            (f::THUMB_WHEEL, 1) => Ok(vec![0, self.thumb]),
            (f::THUMB_WHEEL, 2) => {
                self.thumb = p(1);
                Ok(vec![])
            }
            (f::K375S_FN_INVERSION, 0) => Ok(vec![0, self.fn_swap]),
            (f::K375S_FN_INVERSION, 1) => {
                self.fn_swap = p(1);
                Ok(vec![])
            }
            (f::DISABLE_KEYS, 0) => Ok(vec![self.disable_caps]),
            (f::DISABLE_KEYS, 1) => Ok(vec![self.disabled]),
            (f::DISABLE_KEYS, 2) => {
                self.disabled = p(0) & self.disable_caps;
                Ok(vec![])
            }
            (f::CHANGE_HOST, 0) => Ok(vec![self.hosts.0, self.hosts.1]),
            (f::CHANGE_HOST, 1) => {
                self.hosts.1 = p(0);
                Ok(vec![])
            }
            (f::HOSTS_INFO, 0) => Ok(vec![0x01, 0, self.hosts.0, self.hosts.1]),
            (f::HOSTS_INFO, 1) => {
                let name = self.host_names.get(p(0) as usize).ok_or(0x02u8)?;
                Ok(vec![p(0), !name.is_empty() as u8, 0, 1, name.len() as u8, 24])
            }
            (f::HOSTS_INFO, 3) => {
                let name = self.host_names.get(p(0) as usize).ok_or(0x02u8)?;
                let mut r = vec![p(0), p(1)];
                r.extend(name.as_bytes().iter().skip(p(1) as usize).take(14));
                Ok(r)
            }
            (f::BACKLIGHT2, 0) => Ok(self.backlight.to_vec()),
            (f::BACKLIGHT2, 1) => {
                self.backlight[0] = p(0);
                self.backlight[1] = p(1);
                self.backlight[5] = p(3);
                for i in 4..10 {
                    self.backlight[i + 2] = p(i);
                }
                Ok(vec![])
            }
            (f::BACKLIGHT2, 2) => Ok(vec![8]),
            (f::ONBOARD_PROFILES, 0) => Ok(vec![1, 1, 1, self.profiles, 0, 11, 0, 16, 1, 0]),
            (f::ONBOARD_PROFILES, 1) => {
                self.onboard_mode = p(0);
                Ok(vec![])
            }
            (f::ONBOARD_PROFILES, 2) => Ok(vec![self.onboard_mode]),
            (f::ONBOARD_PROFILES, 3) => {
                self.profile = p(1).clamp(1, self.profiles);
                Ok(vec![])
            }
            (f::ONBOARD_PROFILES, 4) => Ok(vec![0, self.profile]),
            (f::COLOR_LED_EFFECTS, 0) => Ok(vec![self.zones.len() as u8]),
            (f::COLOR_LED_EFFECTS, 1) => {
                let (loc, _, _) = self.zones.get(p(0) as usize).ok_or(0x02u8)?;
                let l = be(*loc);
                Ok(vec![p(0), l[0], l[1], 4, 1])
            }
            (f::COLOR_LED_EFFECTS, 2) => {
                let id: u16 = [0x00, 0x01, 0x03, 0x0A].get(p(1) as usize).copied().ok_or(0x02u8)?;
                let i = be(id);
                Ok(vec![p(0), p(1), i[0], i[1]])
            }
            (f::COLOR_LED_EFFECTS, 3) => {
                let z = self.zones.get_mut(p(0) as usize).ok_or(0x02u8)?;
                z.1 = p(1);
                for i in 0..10 {
                    z.2[i] = p(2 + i);
                }
                Ok(vec![])
            }
            (f::COLOR_LED_EFFECTS, 0x0E) => {
                let (_, e, params) = self.zones.get(p(0) as usize).ok_or(0x02u8)?;
                let mut r = vec![p(0), *e];
                r.extend(params);
                Ok(r)
            }
            (f::REPROG_CONTROLS_V4, 0) => Ok(vec![self.controls.len() as u8]),
            (f::REPROG_CONTROLS_V4, 1) => {
                let (cid, flags, group, mask, _) = *self.controls.get(p(0) as usize).ok_or(0x02u8)?;
                let c = be(cid);
                Ok(vec![c[0], c[1], c[0], c[1], flags, p(0), group, mask, 0])
            }
            (f::REPROG_CONTROLS_V4, 2) => {
                let cid = u16::from_be_bytes([p(0), p(1)]);
                let c = self.controls.iter().find(|c| c.0 == cid).ok_or(0x02u8)?;
                let r = be(c.4);
                Ok(vec![p(0), p(1), 0, r[0], r[1]])
            }
            (f::REPROG_CONTROLS_V4, 3) => {
                let cid = u16::from_be_bytes([p(0), p(1)]);
                let target = u16::from_be_bytes([p(3), p(4)]);
                let c = self.controls.iter_mut().find(|c| c.0 == cid).ok_or(0x02u8)?;
                if target != 0 {
                    c.4 = target;
                }
                Ok(vec![p(0), p(1), 0, p(3), p(4)])
            }
            _ => Err(0x07),
        }
    }
}

pub struct SimReceiver {
    kind: ReceiverKind,
    serial: [u8; 4],
    slots: Vec<Option<SimDevice>>,
}

pub struct SimNode {
    receiver: Option<SimReceiver>,
    direct: Option<SimDevice>,
    queue: VecDeque<Vec<u8>>,
}

fn pad(mut r: Vec<u8>, len: usize) -> Vec<u8> {
    r.resize(len, 0);
    r
}

impl SimNode {
    pub fn receiver(kind: ReceiverKind, devices: Vec<SimDevice>) -> Self {
        let mut slots: Vec<Option<SimDevice>> = devices.into_iter().map(Some).collect();
        slots.resize(kind.slots() as usize, None);
        Self { receiver: Some(SimReceiver { kind, serial: [0xC0, 0xFF, 0xEE, 0x01], slots }), direct: None, queue: VecDeque::new() }
    }

    pub fn direct(device: SimDevice) -> Self {
        Self { receiver: None, direct: Some(device), queue: VecDeque::new() }
    }

    /// The wireless ids paired to a receiver, as the kernel lists them in sysfs.
    pub fn paired_ids(&self) -> Vec<u16> {
        self.receiver.iter().flat_map(|rx| rx.slots.iter().flatten().map(|d| d.wpid)).collect()
    }

    fn reply_short(&mut self, bytes: &[u8]) {
        self.queue.push_back(pad(bytes.to_vec(), 7));
    }

    fn reply_long(&mut self, bytes: &[u8]) {
        self.queue.push_back(pad(bytes.to_vec(), 20));
    }

    fn register(&mut self, sub: u8, addr: u8, p: &[u8]) {
        let Some(rx) = self.receiver.as_mut() else { return };
        let err = |code: u8| vec![0x10, 0xFF, 0x8F, sub, addr, code];
        let kind = rx.kind;
        let mut out: Vec<Vec<u8>> = Vec::new();
        match (sub, addr) {
            (SET_REGISTER, 0x00) => out.push(vec![0x10, 0xFF, sub, addr]),
            (SET_REGISTER, 0x02) => {
                out.push(vec![0x10, 0xFF, sub, addr]);
                for (i, d) in rx.slots.iter().enumerate() {
                    if let Some(d) = d {
                        let flags = d.receiver_code | if d.online { 0 } else { 0x40 };
                        let w = d.wpid.to_le_bytes();
                        out.push(vec![0x10, i as u8 + 1, 0x41, flags, w[0], w[1]]);
                    }
                }
            }
            (SET_REGISTER, 0xB2) => {
                out.push(vec![0x10, 0xFF, sub, addr]);
                match p.first() {
                    Some(0x01) => out.push(vec![0x10, 0xFF, 0x4A, 0x01, 0x00]),
                    Some(0x02) => out.push(vec![0x10, 0xFF, 0x4A, 0x00, 0x00]),
                    Some(0x03) => {
                        let slot = p.get(1).copied().unwrap_or(0);
                        if let Some(s) = rx.slots.get_mut(slot as usize - 1) {
                            *s = None;
                        }
                        out.push(vec![0x10, slot, 0x40, 0x02]);
                    }
                    _ => {}
                }
            }
            (SET_LONG_REGISTER, 0xC1) if p.first() == Some(&0x03) => {
                let slot = p.get(1).copied().unwrap_or(0);
                if let Some(s) = rx.slots.get_mut(slot as usize - 1) {
                    *s = None;
                }
                out.push(vec![0x11, 0xFF, sub, addr]);
                out.push(vec![0x10, slot, 0x40, 0x02]);
            }
            (GET_LONG_REGISTER, 0xB5) => {
                let a = p.first().copied().unwrap_or(0);
                let mut r = vec![0x11, 0xFF, sub, addr, a];
                let slot_dev = |slot: usize| rx.slots.get(slot).and_then(|d| d.as_ref());
                match a {
                    0x03 | 0xFB => {
                        r.extend(rx.serial);
                        r.extend([0, kind.slots()]);
                    }
                    0x20..=0x25 | 0x30..=0x35 | 0x40..=0x45 if kind != ReceiverKind::Bolt => {
                        let Some(d) = slot_dev((a & 0x0F) as usize) else {
                            out.push(err(0x03));
                            self.flush(out);
                            return;
                        };
                        match a & 0xF0 {
                            0x20 => r.extend([0, 8, (d.wpid >> 8) as u8, d.wpid as u8, 0, 0, d.receiver_code]),
                            0x30 => r.extend(d.serial),
                            _ => {
                                r.push(d.name.len() as u8);
                                r.extend(d.name.as_bytes());
                            }
                        }
                    }
                    0x51..=0x56 | 0x61..=0x66 if kind == ReceiverKind::Bolt => {
                        let Some(d) = slot_dev((a & 0x0F) as usize - 1) else {
                            out.push(err(0x03));
                            self.flush(out);
                            return;
                        };
                        if a & 0xF0 == 0x50 {
                            r.push(d.receiver_code);
                            r.extend(d.wpid.to_le_bytes());
                            r.extend(d.serial);
                        } else {
                            r.extend([0x01, d.name.len() as u8]);
                            r.extend(d.name.as_bytes());
                        }
                    }
                    _ => {
                        out.push(err(0x02));
                        self.flush(out);
                        return;
                    }
                }
                out.push(r);
            }
            _ => out.push(err(0x02)),
        }
        self.flush(out);
    }

    fn flush(&mut self, out: Vec<Vec<u8>>) {
        for r in out {
            if r[0] == 0x10 { self.reply_short(&r) } else { self.reply_long(&r) }
        }
    }
}

impl Io for SimNode {
    fn write(&mut self, r: &[u8]) -> io::Result<()> {
        if r.len() < 7 {
            return Ok(());
        }
        let (dev, b2, b3) = (r[1], r[2], r[3]);
        let params = r[4..].to_vec();
        if dev == 0xFF && self.receiver.is_some() && (0x80..=0x83).contains(&b2) {
            self.register(b2, b3, &params);
            return Ok(());
        }
        let device = match (dev, self.receiver.as_mut(), self.direct.as_mut()) {
            (0xFF, _, Some(d)) => Some(d),
            (slot @ 1..=6, Some(rx), _) => rx.slots.get_mut(slot as usize - 1).and_then(|s| s.as_mut()),
            _ => None,
        };
        let reply = match device {
            Some(d) if d.online => Some(d.handle(b2, b3 >> 4, &params)),
            _ => None,
        };
        match reply {
            Some(Ok(p)) => {
                let mut out = vec![0x11, dev, b2, b3];
                out.extend(p);
                self.reply_long(&out);
            }
            Some(Err(code)) => self.reply_long(&[0x11, dev, 0xFF, b2, b3, code]),
            None => self.reply_short(&[0x10, dev, 0x8F, b2, b3, 0x09]),
        }
        Ok(())
    }

    fn read(&mut self, timeout_ms: i32) -> io::Result<Option<Vec<u8>>> {
        if let Some(r) = self.queue.pop_front() {
            return Ok(Some(r));
        }
        if timeout_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(timeout_ms.min(10) as u64));
        }
        Ok(None)
    }
}

/// Simulated nodes for a comma-separated list of models. A `:off` suffix makes
/// that device paired but switched off.
pub fn nodes(spec: &str) -> Vec<(NodeInfo, SimNode)> {
    let mut bolt = Vec::new();
    let mut lightspeed = Vec::new();
    let mut out = Vec::new();
    for item in spec.split(',').map(str::trim).filter(|s| !s.is_empty() && !["denied", "gearbench"].contains(s)) {
        let (name, off) = match item.strip_suffix(":off") {
            Some(n) => (n, true),
            None => (item, false),
        };
        let Some(mut d) = SimDevice::model(name) else {
            eprintln!("logi: unknown mock device {name}");
            continue;
        };
        d.online = !off;
        match name {
            "g502x" => lightspeed.push(d),
            "mx-anywhere-3s" => {
                let info = NodeInfo {
                    path: PathBuf::from("/mock/bluetooth"),
                    name: d.name.to_string(),
                    bus: BUS_BLUETOOTH,
                    product: d.wpid,
                    short: false,
                    long: true,
                };
                out.push((info, SimNode::direct(d)));
            }
            _ => bolt.push(d),
        }
    }
    let rx = |path: &str, pid: u16, kind: ReceiverKind, devs: Vec<SimDevice>| {
        let info =
            NodeInfo { path: PathBuf::from(path), name: kind.label().into(), bus: BUS_USB, product: pid, short: true, long: true };
        (info, SimNode::receiver(kind, devs))
    };
    if !bolt.is_empty() {
        out.insert(0, rx("/mock/bolt", 0xC548, ReceiverKind::Bolt, bolt));
    }
    if !lightspeed.is_empty() {
        out.push(rx("/mock/lightspeed", 0xC547, ReceiverKind::Lightspeed, lightspeed));
    }
    out
}
