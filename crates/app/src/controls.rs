//! Which camera action a mouse button starts. Blender's controls, plus Alt + left
//! drag for touchpads without a middle button.

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
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

pub fn drag_for(button: Button, modifiers: Modifiers) -> Option<Drag> {
    let camera_button = match button {
        Button::Middle => true,
        Button::Left => modifiers.alt,
        Button::Other => false,
    };
    if !camera_button {
        None
    } else if modifiers.shift {
        Some(Drag::Pan)
    } else if modifiers.ctrl {
        Some(Drag::Zoom)
    } else {
        Some(Drag::Orbit)
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
        ctrl: false,
        alt: false,
    };
    const CTRL: Modifiers = Modifiers {
        shift: false,
        ctrl: true,
        alt: false,
    };
    const ALT: Modifiers = Modifiers {
        shift: false,
        ctrl: false,
        alt: true,
    };

    #[test]
    fn middle_button_works_like_blender() {
        assert_eq!(drag_for(Button::Middle, NONE), Some(Drag::Orbit));
        assert_eq!(drag_for(Button::Middle, SHIFT), Some(Drag::Pan));
        assert_eq!(drag_for(Button::Middle, CTRL), Some(Drag::Zoom));
    }

    #[test]
    fn alt_left_stands_in_for_the_middle_button() {
        assert_eq!(drag_for(Button::Left, ALT), Some(Drag::Orbit));
        let alt_shift = Modifiers { shift: true, ..ALT };
        assert_eq!(drag_for(Button::Left, alt_shift), Some(Drag::Pan));
        let alt_ctrl = Modifiers { ctrl: true, ..ALT };
        assert_eq!(drag_for(Button::Left, alt_ctrl), Some(Drag::Zoom));
    }

    #[test]
    fn plain_left_and_other_buttons_leave_the_camera_alone() {
        assert_eq!(drag_for(Button::Left, NONE), None);
        assert_eq!(drag_for(Button::Left, SHIFT), None);
        assert_eq!(drag_for(Button::Other, ALT), None);
    }
}
