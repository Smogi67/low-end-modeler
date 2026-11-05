//! Which camera action a mouse button starts. Chosen by the owner: right drag turns
//! and middle drag slides, with no keys held. Plain left stays free for selecting
//! (Stage 4), and Alt + left drag covers touchpads.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drag {
    Orbit,
    Pan,
    Zoom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

pub fn drag_for(button: Button, modifiers: Modifiers) -> Option<Drag> {
    match button {
        Button::Right => Some(Drag::Orbit),
        Button::Middle if modifiers.ctrl => Some(Drag::Zoom),
        Button::Middle => Some(Drag::Pan),
        Button::Left if modifiers.alt && modifiers.shift => Some(Drag::Pan),
        Button::Left if modifiers.alt && modifiers.ctrl => Some(Drag::Zoom),
        Button::Left if modifiers.alt => Some(Drag::Orbit),
        Button::Left | Button::Other => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Modifiers = Modifiers {
        shift: false,
        ctrl: false,
        alt: false,
    };
    const SHIFT: Modifiers = Modifiers {
        shift: true,
        ..NONE
    };
    const CTRL: Modifiers = Modifiers { ctrl: true, ..NONE };
    const ALT: Modifiers = Modifiers { alt: true, ..NONE };

    #[test]
    fn mouse_buttons_work_without_any_keys() {
        assert_eq!(drag_for(Button::Right, NONE), Some(Drag::Orbit));
        assert_eq!(drag_for(Button::Middle, NONE), Some(Drag::Pan));
    }

    #[test]
    fn ctrl_with_the_middle_button_zooms() {
        assert_eq!(drag_for(Button::Middle, CTRL), Some(Drag::Zoom));
        assert_eq!(drag_for(Button::Middle, SHIFT), Some(Drag::Pan));
    }

    #[test]
    fn alt_left_still_works_on_touchpads() {
        assert_eq!(drag_for(Button::Left, ALT), Some(Drag::Orbit));
        let alt_shift = Modifiers { shift: true, ..ALT };
        assert_eq!(drag_for(Button::Left, alt_shift), Some(Drag::Pan));
        let alt_ctrl = Modifiers { ctrl: true, ..ALT };
        assert_eq!(drag_for(Button::Left, alt_ctrl), Some(Drag::Zoom));
    }

    #[test]
    fn plain_left_stays_free_for_selecting() {
        assert_eq!(drag_for(Button::Left, NONE), None);
        assert_eq!(drag_for(Button::Left, SHIFT), None);
        assert_eq!(drag_for(Button::Other, ALT), None);
    }
}
