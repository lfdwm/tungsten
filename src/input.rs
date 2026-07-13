use sdl3::{
    EventPump,
    event::Event,
    keyboard::{Keycode, Scancode},
};

pub(crate) struct InputState;

impl InputState {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn poll(&mut self, events: &mut EventPump, controls_enabled: bool) -> FrameInput {
        let mut input = FrameInput::default();

        for event in events.poll_iter() {
            input.record_event(event, controls_enabled);
        }

        if controls_enabled {
            input.set_held_scancodes(events.keyboard_state().pressed_scancodes());
        }

        input
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct FrameInput {
    quit_requested: bool,
    key_presses: Vec<Keycode>,
    held_scancodes: Vec<Scancode>,
    mouse_delta: [f32; 2],
    wheel_delta: f32,
}

impl FrameInput {
    pub(crate) fn quit_requested(&self) -> bool {
        self.quit_requested
    }

    pub(crate) fn key_pressed(&self, keycode: Keycode) -> bool {
        self.key_presses.contains(&keycode)
    }

    pub(crate) fn scancode_held(&self, scancode: Scancode) -> bool {
        self.held_scancodes.contains(&scancode)
    }

    pub(crate) fn mouse_delta(&self) -> [f32; 2] {
        self.mouse_delta
    }

    pub(crate) fn wheel_delta(&self) -> f32 {
        self.wheel_delta
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        key_presses: &[Keycode],
        held_scancodes: &[Scancode],
        mouse_delta: [f32; 2],
        wheel_delta: f32,
        quit_requested: bool,
    ) -> Self {
        Self {
            quit_requested,
            key_presses: key_presses.to_vec(),
            held_scancodes: held_scancodes.to_vec(),
            mouse_delta,
            wheel_delta,
        }
    }

    fn record_event(&mut self, event: Event, controls_enabled: bool) {
        match event {
            Event::Quit { .. }
            | Event::KeyDown {
                keycode: Some(Keycode::Escape),
                ..
            } => {
                self.quit_requested = true;
            }
            Event::KeyDown {
                keycode: Some(keycode),
                repeat: false,
                ..
            } if controls_enabled => {
                self.push_key_press(keycode);
            }
            Event::MouseMotion { xrel, yrel, .. } if controls_enabled => {
                self.mouse_delta[0] += xrel;
                self.mouse_delta[1] += yrel;
            }
            Event::MouseWheel { y, .. } if controls_enabled => {
                self.wheel_delta += y;
            }
            _ => {}
        }
    }

    fn push_key_press(&mut self, keycode: Keycode) {
        if !self.key_presses.contains(&keycode) {
            self.key_presses.push(keycode);
        }
    }

    fn set_held_scancodes(&mut self, scancodes: impl IntoIterator<Item = Scancode>) {
        self.held_scancodes.clear();
        for scancode in scancodes {
            if !self.held_scancodes.contains(&scancode) {
                self.held_scancodes.push(scancode);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdl3::{
        keyboard::Mod,
        mouse::{MouseState, MouseWheelDirection},
    };

    fn key_down(keycode: Keycode, repeat: bool) -> Event {
        Event::KeyDown {
            timestamp: 0,
            window_id: 0,
            keycode: Some(keycode),
            scancode: None,
            keymod: Mod::NOMOD,
            repeat,
            which: 0,
            raw: 0,
        }
    }

    #[test]
    fn records_non_repeat_key_presses() {
        let mut input = FrameInput::default();
        input.record_event(key_down(Keycode::F3, false), true);
        input.record_event(key_down(Keycode::F11, true), true);

        assert!(input.key_pressed(Keycode::F3));
        assert!(!input.key_pressed(Keycode::F11));
    }

    #[test]
    fn ignores_control_events_when_disabled_but_keeps_quit() {
        let mut input = FrameInput::default();
        input.record_event(key_down(Keycode::F3, false), false);
        input.record_event(key_down(Keycode::Escape, false), false);

        assert!(!input.key_pressed(Keycode::F3));
        assert!(input.quit_requested());
    }

    #[test]
    fn syncs_held_scancodes() {
        let mut input = FrameInput::default();
        input.set_held_scancodes([Scancode::W, Scancode::LShift, Scancode::W]);

        assert!(input.scancode_held(Scancode::W));
        assert!(input.scancode_held(Scancode::LShift));
        assert!(!input.scancode_held(Scancode::S));
        assert_eq!(input.held_scancodes.len(), 2);
    }

    #[test]
    fn accumulates_mouse_and_wheel_motion() {
        let mut input = FrameInput::default();
        input.record_event(
            Event::MouseMotion {
                timestamp: 0,
                window_id: 0,
                which: 0,
                mousestate: MouseState::from_sdl_state(0),
                x: 0.0,
                y: 0.0,
                xrel: 2.5,
                yrel: -1.0,
            },
            true,
        );
        input.record_event(
            Event::MouseMotion {
                timestamp: 0,
                window_id: 0,
                which: 0,
                mousestate: MouseState::from_sdl_state(0),
                x: 0.0,
                y: 0.0,
                xrel: 1.0,
                yrel: 3.0,
            },
            true,
        );
        input.record_event(
            Event::MouseWheel {
                timestamp: 0,
                window_id: 0,
                which: 0,
                x: 0.0,
                y: 2.0,
                direction: MouseWheelDirection::Normal,
                mouse_x: 0.0,
                mouse_y: 0.0,
                integer_x: 0,
                integer_y: 2,
            },
            true,
        );

        assert_eq!(input.mouse_delta(), [3.5, 2.0]);
        assert_eq!(input.wheel_delta(), 2.0);
    }
}
