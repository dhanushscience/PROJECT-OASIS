//! Port of `Input/ArbiterConfig.cs`: the user's button mapping and gesture timings.

use std::time::Duration;

use crate::primitives::{ButtonId, ButtonMask};

const fn ms(v: u64) -> Duration {
    Duration::from_millis(v)
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArbiterConfig {
    pub double_click_window: Duration,
    pub volume_push_to_talk_floor: Duration,
    pub hold2x_threshold: Duration,
    pub push_to_talk_hold: Duration,
    pub ai_hold_duration: Duration,
    pub volume_up: Option<ButtonId>,
    pub volume_down: Option<ButtonId>,
    pub hold2x: Option<ButtonId>,
    pub push_to_talk_mask: ButtonMask,
    pub voice_enabled: bool,
    pub ai_enabled: bool,
    pub ai_trigger_mask: ButtonMask,
    pub show_overlay_labels: bool,
    pub show_desktop: Option<ButtonId>,
    pub show_desktop_hold: Duration,
}

impl ArbiterConfig {
    pub const SHOW_DESKTOP_HOLD_MIN_MS: u64 = 400;
    pub const SHOW_DESKTOP_HOLD_MAX_MS: u64 = 3000;

    /// Matches the C# `ProfileSettings` defaults: voice and AI off until the user enables them.
    pub fn product_defaults() -> ArbiterConfig {
        ArbiterConfig {
            double_click_window: ms(300),
            volume_push_to_talk_floor: ms(700),
            hold2x_threshold: ms(250),
            push_to_talk_hold: ms(200),
            ai_hold_duration: ms(5000),
            volume_up: Some(ButtonId::ONE),
            volume_down: Some(ButtonId::THREE),
            hold2x: Some(ButtonId::TWO),
            push_to_talk_mask: ButtonMask::TWO,
            voice_enabled: false,
            ai_enabled: false,
            ai_trigger_mask: ButtonMask::TWO,
            show_overlay_labels: true,
            show_desktop: Some(ButtonId::THREE),
            show_desktop_hold: ms(800),
        }
    }

    /// The owner's own pad: BTN 1 = PTT + volume down, BTN 2 = AI + volume up, BTN 3 = hold-2×.
    /// Labels off.
    pub fn user_pad() -> ArbiterConfig {
        ArbiterConfig {
            volume_up: Some(ButtonId::TWO),
            volume_down: Some(ButtonId::ONE),
            hold2x: Some(ButtonId::THREE),
            push_to_talk_mask: ButtonMask::ONE,
            voice_enabled: true,
            ai_enabled: true,
            ai_trigger_mask: ButtonMask::TWO,
            show_overlay_labels: false,
            ..ArbiterConfig::product_defaults()
        }
    }

    pub fn is_volume_button(&self, button: ButtonId) -> bool {
        self.volume_up == Some(button) || self.volume_down == Some(button)
    }

    pub fn is_hold2x_button(&self, button: ButtonId) -> bool {
        self.hold2x == Some(button)
    }

    pub fn is_show_desktop_button(&self, button: ButtonId) -> bool {
        self.show_desktop == Some(button)
    }

    /// Clamped so a slow tap never shows the desktop and a hold is still reachable.
    pub fn show_desktop_delay(&self) -> Duration {
        self.show_desktop_hold.clamp(
            ms(Self::SHOW_DESKTOP_HOLD_MIN_MS),
            ms(Self::SHOW_DESKTOP_HOLD_MAX_MS),
        )
    }

    pub fn ai_owns_press_path(&self) -> bool {
        self.ai_enabled && !self.ai_trigger_mask.is_empty()
    }

    pub fn ai_trigger_held(&self, held: ButtonMask) -> bool {
        self.ai_owns_press_path() && (held & self.ai_trigger_mask) == self.ai_trigger_mask
    }

    /// No floor: the volume buttons are also PREV / JUMP, and two seeks in a row must stay two
    /// seeks. A long window (it used to be at least 700 ms) turned "skip 10 s" into volume.
    pub fn double_click_delay(&self) -> Duration {
        self.double_click_window.clamp(ms(200), ms(1000))
    }

    /// How long a push-to-talk hold must last before the mic opens. On a button that is also a
    /// volume button the hold must be at least `volume_push_to_talk_floor`, so a quick
    /// double-tap for volume never opens the mic.
    pub fn push_to_talk_delay(&self, volume_button: bool) -> Duration {
        let mut d = self.push_to_talk_hold.clamp(ms(200), ms(2000));
        if volume_button {
            d = d.max(self.volume_push_to_talk_floor);
        }
        d
    }
}

impl Default for ArbiterConfig {
    fn default() -> Self {
        ArbiterConfig::product_defaults()
    }
}
