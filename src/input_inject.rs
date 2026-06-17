use evdev::uinput::VirtualDeviceBuilder;
use evdev::{AttributeSet, EventType, InputEvent, Key, RelativeAxisType};

pub struct InputInjector {
    keyboard: Option<evdev::uinput::VirtualDevice>,
    mouse: Option<evdev::uinput::VirtualDevice>,
}

impl InputInjector {
    pub fn new() -> Self {
        Self {
            keyboard: None,
            mouse: None,
        }
    }

    pub fn setup(&mut self) -> anyhow::Result<()> {
        // Create virtual keyboard with all standard keys (excluding mouse buttons)
        let mut keys = AttributeSet::<Key>::new();
        for code in 0..0x2ff_u16 {
            let key = Key::new(code);
            if key == Key::BTN_LEFT
                || key == Key::BTN_RIGHT
                || key == Key::BTN_MIDDLE
                || key == Key::BTN_SIDE
                || key == Key::BTN_EXTRA
            {
                continue;
            }
            keys.insert(key);
        }

        let keyboard = VirtualDeviceBuilder::new()?
            .name("BlueCross Virtual Keyboard")
            .with_keys(&keys)?
            .build()?;
        log::info!("Virtual keyboard created");
        self.keyboard = Some(keyboard);

        // Create virtual mouse
        let mut buttons = AttributeSet::<Key>::new();
        buttons.insert(Key::BTN_LEFT);
        buttons.insert(Key::BTN_RIGHT);
        buttons.insert(Key::BTN_MIDDLE);
        buttons.insert(Key::BTN_SIDE);
        buttons.insert(Key::BTN_EXTRA);

        let mut rel_axes = AttributeSet::<RelativeAxisType>::new();
        rel_axes.insert(RelativeAxisType::REL_X);
        rel_axes.insert(RelativeAxisType::REL_Y);
        rel_axes.insert(RelativeAxisType::REL_WHEEL);
        rel_axes.insert(RelativeAxisType::REL_HWHEEL);

        let mouse = VirtualDeviceBuilder::new()?
            .name("BlueCross Virtual Mouse")
            .with_keys(&buttons)?
            .with_relative_axes(&rel_axes)?
            .build()?;
        log::info!("Virtual mouse created");
        self.mouse = Some(mouse);

        Ok(())
    }

    pub fn inject_key(&mut self, code: u16, value: i32) {
        if let Some(ref mut kb) = self.keyboard {
            let events = [
                InputEvent::new(EventType::KEY, code, value),
                InputEvent::new(EventType::SYNCHRONIZATION, 0, 0),
            ];
            let _ = kb.emit(&events);
        }
    }

    pub fn inject_mouse_move(&mut self, dx: i32, dy: i32) {
        if let Some(ref mut mouse) = self.mouse {
            let mut events = Vec::with_capacity(3);
            if dx != 0 {
                events.push(InputEvent::new(
                    EventType::RELATIVE,
                    RelativeAxisType::REL_X.0,
                    dx,
                ));
            }
            if dy != 0 {
                events.push(InputEvent::new(
                    EventType::RELATIVE,
                    RelativeAxisType::REL_Y.0,
                    dy,
                ));
            }
            if !events.is_empty() {
                events.push(InputEvent::new(EventType::SYNCHRONIZATION, 0, 0));
                let _ = mouse.emit(&events);
            }
        }
    }

    pub fn inject_mouse_button(&mut self, button: u16, value: i32) {
        if let Some(ref mut mouse) = self.mouse {
            let events = [
                InputEvent::new(EventType::KEY, button, value),
                InputEvent::new(EventType::SYNCHRONIZATION, 0, 0),
            ];
            let _ = mouse.emit(&events);
        }
    }

    pub fn inject_mouse_scroll(&mut self, dx: i32, dy: i32) {
        if let Some(ref mut mouse) = self.mouse {
            let mut events = Vec::with_capacity(3);
            if dy != 0 {
                events.push(InputEvent::new(
                    EventType::RELATIVE,
                    RelativeAxisType::REL_WHEEL.0,
                    dy,
                ));
            }
            if dx != 0 {
                events.push(InputEvent::new(
                    EventType::RELATIVE,
                    RelativeAxisType::REL_HWHEEL.0,
                    dx,
                ));
            }
            if !events.is_empty() {
                events.push(InputEvent::new(EventType::SYNCHRONIZATION, 0, 0));
                let _ = mouse.emit(&events);
            }
        }
    }

    pub fn close(&mut self) {
        self.keyboard = None;
        self.mouse = None;
    }
}
