//! Linux legacy joystick (`/dev/input/js*`) backend.
//!
//! No ioctls, privileged code or native library is required: the device is read
//! as a non-blocking byte stream in user space. Missing devices or permissions
//! are silent; keyboard control remains available.

use super::{buttons, Backend, RawPad, MAX_PADS};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read},
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    time::{Duration, Instant},
};

const BUTTON_EVENT: u8 = 1;
const AXIS_EVENT: u8 = 2;
const INIT_FLAG: u8 = 0x80;
const HAT_THRESHOLD: i16 = 16_000;
const RESCAN: Duration = Duration::from_secs(2);

#[derive(Default)]
pub struct Platform {
    devices: std::vec::Vec<Device>,
    next_scan: Option<Instant>,
}

impl Platform {
    pub fn new() -> Self {
        Self::default()
    }
}

struct Device {
    path: PathBuf,
    file: File,
    buttons: u32,
    axes: [i16; 8],
    pending: [u8; 7],
    pending_len: usize,
}

impl Device {
    fn open(path: PathBuf) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path)?;
        Ok(Self {
            path,
            file,
            buttons: 0,
            axes: [0; 8],
            pending: [0; 7],
            pending_len: 0,
        })
    }

    /// Read all currently buffered events. Returns false on disconnect.
    fn pump(&mut self) -> bool {
        let mut buffer = [0u8; 8 * 32 + 7];
        let carried = self.pending_len;
        buffer[..carried].copy_from_slice(&self.pending[..carried]);
        loop {
            match self.file.read(&mut buffer[carried..]) {
                Ok(0) => return false,
                Ok(count) => {
                    let total = carried + count;
                    let mut offset = 0;
                    while offset + 8 <= total {
                        let event = &buffer[offset..offset + 8];
                        let value = i16::from_le_bytes([event[4], event[5]]);
                        let kind = event[6] & !INIT_FLAG;
                        let number = event[7];
                        match kind {
                            BUTTON_EVENT if number < 32 => {
                                let bit = 1u32 << number;
                                if value != 0 {
                                    self.buttons |= bit;
                                } else {
                                    self.buttons &= !bit;
                                }
                            }
                            AXIS_EVENT if number < self.axes.len() as u8 => {
                                self.axes[number as usize] = value;
                            }
                            _ => {}
                        }
                        offset += 8;
                    }
                    // Preserve a partial event for the next poll rather than
                    // desynchronising the fixed-size stream.
                    self.pending_len = total - offset;
                    self.pending[..self.pending_len].copy_from_slice(&buffer[offset..total]);
                    return true;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return true,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return false,
            }
        }
    }

    fn raw(&self) -> RawPad {
        let mut mask = 0;
        for (native, abstract_bit) in [
            (0, buttons::A),
            (1, buttons::B),
            (2, buttons::X),
            (3, buttons::Y),
            (4, buttons::LEFT_SHOULDER),
            (5, buttons::RIGHT_SHOULDER),
            (6, buttons::BACK),
            (7, buttons::START),
            (9, buttons::LEFT_THUMB),
            (10, buttons::RIGHT_THUMB),
            (11, buttons::DPAD_UP),
            (12, buttons::DPAD_DOWN),
            (13, buttons::DPAD_LEFT),
            (14, buttons::DPAD_RIGHT),
        ] {
            if self.buttons & (1u32 << native) != 0 {
                mask |= abstract_bit;
            }
        }
        let hat = |value: i16| {
            if value < -HAT_THRESHOLD {
                -1
            } else if value > HAT_THRESHOLD {
                1
            } else {
                0
            }
        };
        RawPad {
            connected: true,
            buttons: mask,
            lx: self.axes[0],
            ly: self.axes[1],
            hat_x: hat(self.axes[6]),
            hat_y: hat(self.axes[7]),
        }
    }
}

impl Platform {
    fn scan(&mut self) {
        let mut paths = std::vec::Vec::new();
        if let Ok(entries) = fs::read_dir("/dev/input") {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with("js") && name.len() <= 6 {
                    paths.push(entry.path());
                }
            }
        }
        paths.sort();
        for path in paths {
            if self.devices.len() >= MAX_PADS {
                break;
            }
            if self.devices.iter().any(|device| device.path == path) {
                continue;
            }
            if let Ok(device) = Device::open(path) {
                self.devices.push(device);
            }
        }
        self.next_scan = Some(Instant::now() + RESCAN);
    }
}

impl Backend for Platform {
    fn poll(&mut self, pads: &mut [RawPad; MAX_PADS]) -> usize {
        let now = Instant::now();
        if self.next_scan.is_none_or(|next| now >= next) {
            self.scan();
        }
        let mut index = 0;
        let before = self.devices.len();
        let mut alive = std::vec::Vec::with_capacity(before);
        for mut device in self.devices.drain(..) {
            if !device.pump() {
                continue;
            }
            if index < MAX_PADS {
                pads[index] = device.raw();
                index += 1;
            }
            alive.push(device);
        }
        let removed = alive.len() != before;
        self.devices = alive;
        if removed || self.devices.is_empty() {
            // Retry periodically so plugging a pad in is picked up without a
            // dedicated hotplug thread.
            self.next_scan = Some(now + RESCAN);
        }
        index
    }
}
