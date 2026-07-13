use sdl3::{
    EventPump,
    event::Event,
    keyboard::{Keycode, Scancode},
    mouse::MouseButton,
};

pub struct InputState {
    held_mouse_buttons: Vec<MouseButton>,
}

impl InputState {
    pub fn new() -> Self {
        Self {
            held_mouse_buttons: Vec::new(),
        }
    }

    pub fn poll(&mut self, events: &mut EventPump, controls_enabled: bool) -> FrameInput {
        let mut input = FrameInput::default();

        for event in events.poll_iter() {
            input.record_event(event, controls_enabled, &mut self.held_mouse_buttons);
        }

        if controls_enabled {
            input.set_held_scancodes(events.keyboard_state().pressed_scancodes());
        }
        input.set_held_mouse_buttons(self.held_mouse_buttons.iter().copied());

        input
    }
}

#[derive(Clone, Debug, Default)]
pub struct FrameInput {
    quit_requested: bool,
    key_presses: Vec<Keycode>,
    held_scancodes: Vec<Scancode>,
    held_mouse_buttons: Vec<MouseButton>,
    mouse_presses: Vec<MouseButton>,
    mouse_releases: Vec<MouseButton>,
    mouse_delta: [f32; 2],
    mouse_position: Option<[f32; 2]>,
    wheel_delta: f32,
    text_input: Vec<String>,
}

impl FrameInput {
    pub fn quit_requested(&self) -> bool {
        self.quit_requested
    }

    pub fn key_pressed(&self, keycode: Keycode) -> bool {
        self.key_presses.contains(&keycode)
    }

    pub fn scancode_held(&self, scancode: Scancode) -> bool {
        self.held_scancodes.contains(&scancode)
    }

    pub fn mouse_pressed(&self, button: MouseButton) -> bool {
        self.mouse_presses.contains(&button)
    }

    pub fn mouse_released(&self, button: MouseButton) -> bool {
        self.mouse_releases.contains(&button)
    }

    pub fn mouse_held(&self, button: MouseButton) -> bool {
        self.held_mouse_buttons.contains(&button)
    }

    pub fn mouse_delta(&self) -> [f32; 2] {
        self.mouse_delta
    }

    pub fn mouse_position(&self) -> Option<[f32; 2]> {
        self.mouse_position
    }

    pub fn wheel_delta(&self) -> f32 {
        self.wheel_delta
    }

    pub fn text_input(&self) -> &[String] {
        &self.text_input
    }

    #[cfg(test)]
    pub fn for_test(
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
            held_mouse_buttons: Vec::new(),
            mouse_presses: Vec::new(),
            mouse_releases: Vec::new(),
            mouse_delta,
            mouse_position: None,
            wheel_delta,
            text_input: Vec::new(),
        }
    }

    fn record_event(
        &mut self,
        event: Event,
        controls_enabled: bool,
        held_mouse_buttons: &mut Vec<MouseButton>,
    ) {
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
            Event::TextInput { text, .. } if controls_enabled => {
                self.text_input.push(text);
            }
            Event::MouseMotion {
                x, y, xrel, yrel, ..
            } if controls_enabled => {
                self.mouse_delta[0] += xrel;
                self.mouse_delta[1] += yrel;
                self.mouse_position = Some([x, y]);
            }
            Event::MouseButtonDown {
                mouse_btn, x, y, ..
            } if controls_enabled => {
                self.mouse_position = Some([x, y]);
                self.push_mouse_press(mouse_btn);
                push_unique_mouse_button(held_mouse_buttons, mouse_btn);
            }
            Event::MouseButtonUp {
                mouse_btn, x, y, ..
            } if controls_enabled => {
                self.mouse_position = Some([x, y]);
                self.push_mouse_release(mouse_btn);
                held_mouse_buttons.retain(|held| *held != mouse_btn);
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

    fn push_mouse_press(&mut self, button: MouseButton) {
        if !self.mouse_presses.contains(&button) {
            self.mouse_presses.push(button);
        }
    }

    fn push_mouse_release(&mut self, button: MouseButton) {
        if !self.mouse_releases.contains(&button) {
            self.mouse_releases.push(button);
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

    fn set_held_mouse_buttons(&mut self, buttons: impl IntoIterator<Item = MouseButton>) {
        self.held_mouse_buttons.clear();
        for button in buttons {
            push_unique_mouse_button(&mut self.held_mouse_buttons, button);
        }
    }
}

fn push_unique_mouse_button(buttons: &mut Vec<MouseButton>, button: MouseButton) {
    if !buttons.contains(&button) {
        buttons.push(button);
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
        let mut held_mouse_buttons = Vec::new();
        input.record_event(key_down(Keycode::F3, false), true, &mut held_mouse_buttons);
        input.record_event(key_down(Keycode::F11, true), true, &mut held_mouse_buttons);

        assert!(input.key_pressed(Keycode::F3));
        assert!(!input.key_pressed(Keycode::F11));
    }

    #[test]
    fn ignores_control_events_when_disabled_but_keeps_quit() {
        let mut input = FrameInput::default();
        let mut held_mouse_buttons = Vec::new();
        input.record_event(key_down(Keycode::F3, false), false, &mut held_mouse_buttons);
        input.record_event(
            key_down(Keycode::Escape, false),
            false,
            &mut held_mouse_buttons,
        );

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
        let mut held_mouse_buttons = Vec::new();
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
            &mut held_mouse_buttons,
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
            &mut held_mouse_buttons,
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
            &mut held_mouse_buttons,
        );

        assert_eq!(input.mouse_delta(), [3.5, 2.0]);
        assert_eq!(input.mouse_position(), Some([0.0, 0.0]));
        assert_eq!(input.wheel_delta(), 2.0);
    }

    #[test]
    fn records_mouse_button_presses_and_releases() {
        let mut input = FrameInput::default();
        let mut held_mouse_buttons = Vec::new();

        input.record_event(
            Event::MouseButtonDown {
                timestamp: 0,
                window_id: 0,
                which: 0,
                mouse_btn: MouseButton::Left,
                clicks: 1,
                x: 10.0,
                y: 20.0,
            },
            true,
            &mut held_mouse_buttons,
        );
        input.record_event(
            Event::MouseButtonUp {
                timestamp: 0,
                window_id: 0,
                which: 0,
                mouse_btn: MouseButton::Left,
                clicks: 1,
                x: 10.0,
                y: 20.0,
            },
            true,
            &mut held_mouse_buttons,
        );
        input.set_held_mouse_buttons(held_mouse_buttons.iter().copied());

        assert!(input.mouse_pressed(MouseButton::Left));
        assert!(input.mouse_released(MouseButton::Left));
        assert!(!input.mouse_held(MouseButton::Left));
        assert_eq!(input.mouse_position(), Some([10.0, 20.0]));
    }
}
