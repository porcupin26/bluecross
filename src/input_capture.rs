use std::os::fd::AsRawFd;
use std::path::Path;

use evdev::{Device, EventType, Key, RelativeAxisType};
use tokio::sync::mpsc;

const EVIOCGRAB: libc::c_ulong = 0x40044590;

#[derive(Debug, Clone)]
pub enum CaptureEvent {
    Key { code: u16, value: i32 },
    MouseMove { dx: i32, dy: i32 },
    MouseButton { code: u16, value: i32 },
    MouseScroll { dx: i32, dy: i32 },
}

/// Start input capture. Returns (event receiver, list of raw fds for grab/ungrab).
///
/// Each device is converted to an event stream and the grab fd is taken from the
/// *live* stream object, which owns the fd for as long as its reader task runs.
pub fn start_capture(
    screen_width: i32,
    screen_height: i32,
) -> anyhow::Result<(mpsc::Receiver<CaptureEvent>, Vec<i32>)> {
    let (tx, rx) = mpsc::channel(2048);
    let devices = discover_devices()?;

    log::info!(
        "Discovered {} input device(s), screen {}x{}",
        devices.len(),
        screen_width,
        screen_height,
    );

    let mut raw_fds = Vec::with_capacity(devices.len());
    for device in devices {
        let stream = match device.into_event_stream() {
            Ok(s) => s,
            Err(e) => {
                log::error!("Failed to open event stream: {}", e);
                continue;
            }
        };
        raw_fds.push(stream.device().as_raw_fd());
        let tx = tx.clone();
        tokio::spawn(async move {
            if let Err(e) = read_device_events(stream, tx).await {
                log::error!("Device reader error: {}", e);
            }
        });
    }

    Ok((rx, raw_fds))
}

fn discover_devices() -> anyhow::Result<Vec<Device>> {
    let mut devices = Vec::new();
    let mut keyboard_count = 0;
    let mut mouse_count = 0;

    let input_dir = Path::new("/dev/input");
    if !input_dir.exists() {
        anyhow::bail!("/dev/input does not exist");
    }

    let mut entries: Vec<_> = std::fs::read_dir(input_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|s| s.starts_with("event"))
                .unwrap_or(false)
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        let device = match Device::open(&path) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let supported_keys = device.supported_keys();
        let supported_rel = device.supported_relative_axes();

        let mut is_keyboard = false;
        let mut is_mouse = false;

        if let Some(keys) = supported_keys {
            if keys.contains(Key::KEY_A) {
                is_keyboard = true;
            } else if keys.contains(Key::BTN_LEFT) {
                is_mouse = true;
            }
        }

        if !is_keyboard && !is_mouse {
            if let Some(rel) = supported_rel {
                if rel.contains(RelativeAxisType::REL_X) || rel.contains(RelativeAxisType::REL_Y) {
                    is_mouse = true;
                }
            }
        }

        if is_keyboard || is_mouse {
            let name = device.name().unwrap_or("unknown").to_string();
            if is_keyboard {
                keyboard_count += 1;
                log::info!("  Keyboard: {} ({})", name, path.display());
            } else {
                mouse_count += 1;
                log::info!("  Mouse: {} ({})", name, path.display());
            }
            devices.push(device);
        }
    }

    log::info!(
        "Discovered {} keyboard(s) and {} mouse/pointer(s)",
        keyboard_count,
        mouse_count,
    );

    Ok(devices)
}

async fn read_device_events(
    mut stream: evdev::EventStream,
    tx: mpsc::Sender<CaptureEvent>,
) -> anyhow::Result<()> {
    loop {
        let event = stream.next_event().await?;
        let capture_event = match event.event_type() {
            EventType::KEY => {
                let key = Key::new(event.code());
                if key == Key::BTN_LEFT
                    || key == Key::BTN_RIGHT
                    || key == Key::BTN_MIDDLE
                    || key == Key::BTN_SIDE
                    || key == Key::BTN_EXTRA
                {
                    CaptureEvent::MouseButton {
                        code: event.code(),
                        value: event.value(),
                    }
                } else {
                    CaptureEvent::Key {
                        code: event.code(),
                        value: event.value(),
                    }
                }
            }
            EventType::RELATIVE => {
                let axis = RelativeAxisType(event.code());
                if axis == RelativeAxisType::REL_X {
                    CaptureEvent::MouseMove {
                        dx: event.value(),
                        dy: 0,
                    }
                } else if axis == RelativeAxisType::REL_Y {
                    CaptureEvent::MouseMove {
                        dx: 0,
                        dy: event.value(),
                    }
                } else if axis == RelativeAxisType::REL_WHEEL {
                    CaptureEvent::MouseScroll {
                        dx: 0,
                        dy: event.value(),
                    }
                } else if axis == RelativeAxisType::REL_HWHEEL {
                    CaptureEvent::MouseScroll {
                        dx: event.value(),
                        dy: 0,
                    }
                } else {
                    continue;
                }
            }
            _ => continue,
        };

        if tx.send(capture_event).await.is_err() {
            break;
        }
    }
    Ok(())
}

pub fn grab_devices(fds: &[i32]) {
    for &fd in fds {
        unsafe {
            libc::ioctl(fd, EVIOCGRAB as libc::Ioctl, 1 as libc::c_int);
        }
    }
}

pub fn ungrab_devices(fds: &[i32]) {
    for &fd in fds {
        unsafe {
            libc::ioctl(fd, EVIOCGRAB as libc::Ioctl, 0 as libc::c_int);
        }
    }
}
