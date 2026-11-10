//! Whether a physical press belongs to the wheel, decided before the click
//! reaches the application.
//!
//! This is the policy half of the gesture; [`crate::wheel`] is the geometry
//! half, and neither knows what a window is. The Windows layer
//! (`crates/platform-win/src/hook.rs`) feeds raw observations in and acts on
//! what comes back, because a low-level hook has to answer *now*: let the
//! message through, or swallow it before the text editor underneath starts a
//! selection. Fifty milliseconds of polling is too late for that, which is the
//! whole reason a hook exists.
//!
//! The rules that matter here are the ones that keep the desktop honest:
//!
//! * A press is only taken when the trigger's modifiers are down *and* the
//!   button matches. Anything else is passed through untouched.
//! * A press that lands anywhere in the window's frame is passed through.
//!   Measured 2026-11-05 (`examples/alt-drag-probe`): holding `Alt` changes
//!   neither the resize border nor the caption - both still run the system's own
//!   move/size loop with the same deltas as without it, so the frame belongs to
//!   Windows and the wheel has no claim on it.
//! * A swallowed press must end in exactly one of two ways - the wheel picks
//!   (both the down and the up stay swallowed) or the gesture is abandoned (both
//!   are replayed as one ordinary click). Losing the down and keeping the up is
//!   how an application gets a stuck mouse button.

use std::time::{Duration, Instant};

/// How long the trigger has to stay down before the wheel appears: 350ms is
/// long enough to be deliberate, short enough not to feel stuck.
pub const HOLD_MS: u64 = 350;

/// A pointer location in physical pixels, as the hook reports it. No DPI has
/// been applied yet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pt {
    pub x: i32,
    pub y: i32,
}

impl Pt {
    pub fn new(x: i32, y: i32) -> Self {
        Pt { x, y }
    }

    /// Travel between two points, in physical pixels.
    fn distance_to(&self, other: &Pt) -> f64 {
        let dx = (other.x - self.x) as f64;
        let dy = (other.y - self.y) as f64;
        (dx * dx + dy * dy).sqrt()
    }
}

/// The offset from the press point that the wheel is aimed with. The wheel
/// centre is the press point, never the cursor, so this is the one number the
/// renderer needs and the reason a finger can leave the hub and come back.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Offset {
    pub dx: i32,
    pub dy: i32,
    /// Physical pixels from the press point, kept so the caller can log or
    /// compare against the threshold without redoing the maths.
    pub distance: f64,
}

/// Which mouse button the drag starts with. Both references use the left
/// button; the right one is offered because it has to swallow a context menu
/// instead of a selection, and some people find that the safer trade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

/// What the trigger is aimed with: a mouse button, or any key on the keyboard.
/// A keyboard trigger opens the wheel at the pointer's position when the key is
/// held, which is the same gesture with a different finger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerInput {
    Mouse(Button),
    Key(u16),
}

/// Modifier keys the trigger asks for. `win` is here because the settings
/// parser already accepts it; the wheel does not treat it specially.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub alt: bool,
    pub ctrl: bool,
    pub shift: bool,
    pub win: bool,
}

impl Mods {
    /// `Ctrl` alone is the shipped trigger: one key, one button, no chord.
    pub fn ctrl() -> Self {
        Mods {
            ctrl: true,
            ..Default::default()
        }
    }

    /// `Alt` alone, the other half of the old shipped trigger.
    pub fn alt() -> Self {
        Mods {
            alt: true,
            ..Default::default()
        }
    }

    /// Does this modifier set match the trigger? Extra modifiers the user is
    /// carrying (a Ctrl held for a multi-select, say) are not the wheel's
    /// business, so the trigger's own keys must all be down and that is enough.
    pub fn covers(&self, want: &Mods) -> bool {
        (!want.alt || self.alt)
            && (!want.ctrl || self.ctrl)
            && (!want.shift || self.shift)
            && (!want.win || self.win)
    }
}

/// Smallest and largest drag threshold the settings offer, in physical pixels.
/// Below 2 the wheel opens from a shiver; past 40 it stops being a flick and
/// becomes a chore on a laptop touchpad.
pub const THRESHOLD_MIN: i32 = 2;
pub const THRESHOLD_MAX: i32 = 40;
pub const THRESHOLD_DEFAULT: i32 = 8;

/// What the gesture watches for, and what it refuses to touch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trigger {
    pub mods: Mods,
    pub input: TriggerInput,
    pub threshold: i32,
}

impl Default for Trigger {
    fn default() -> Self {
        Trigger {
            mods: Mods::alt(),
            input: TriggerInput::Mouse(Button::Left),
            threshold: THRESHOLD_DEFAULT,
        }
    }
}

impl Trigger {
    /// Clamps a hand-edited threshold into the range the slider produces, the
    /// same way [`crate::wheel::Geometry::new`] guards the radius.
    pub fn clamped(self) -> Self {
        Trigger {
            threshold: self.threshold.clamp(THRESHOLD_MIN, THRESHOLD_MAX),
            ..self
        }
    }
}

/// Everything the hook had to look at before deciding, none of it a fact the
/// wheel can afford to guess about.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PressContext {
    /// The point falls in the window's non-client frame - resize border,
    /// caption, the buttons on it - so Windows owns that drag. Measured: Alt
    /// does not change either one's ownership, so the wheel has to stand down
    /// for the whole frame and not just the outer strip.
    pub on_frame: bool,
    /// The foreground window has the display to itself (a fullscreen exclusive
    /// game). Taking its clicks would break it, and the references agree.
    pub exclusive_foreground: bool,
}

/// The answer to a press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnPress {
    /// Swallow the down and start watching for the drag.
    Take,
    /// Let it through exactly as it is. Every reason not to take a gesture lives
    /// here, and they are all deliberate rather than a fallback.
    Pass,
}

/// The answer to a move while a press is being watched.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OnMove {
    /// Nothing to tell the renderer: still inside the threshold, or no gesture
    /// is in flight.
    Nothing,
    /// The cursor moved while the wheel is open. Carries the offset from the
    /// press point.
    Aim(Offset),
}

/// The answer to a release while a press is being watched.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OnRelease {
    /// The gesture was ours. Swallow the up; the caller picks with this offset.
    Pick(Offset),
    /// The gesture was ours and the user let go inside the hub. Swallow the up;
    /// the caller cancels.
    Cancel,
    /// The travel never reached the threshold, so this was a click the user
    /// meant for an application. It was never taken, so there is nothing to
    /// replay - the down and up both went through normally.
    NotOurs,
    /// The press *was* taken and then abandoned. The caller must put the click
    /// back (design §6.3 rails 2 and 3) before it returns from the hook.
    ReplayClick(Pt),
}

/// One gesture in flight, or nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
struct InFlight {
    press: Pt,
    /// When the press was taken. The hold, not the travel, is what opens.
    since: Instant,
    /// The wheel opened from this press.
    opened: bool,
    /// The press was swallowed by us, so a cancellation owes the application a
    /// replayed click.
    swallowed: bool,
}

/// The gesture state machine. One instance per desktop, owned by the hook's
/// thread; it holds a handful of numbers and no resources.
#[derive(Clone, Debug)]
pub struct Machine {
    trigger: Trigger,
    flight: Option<InFlight>,
}

impl Machine {
    pub fn new(trigger: Trigger) -> Self {
        Machine {
            trigger: trigger.clamped(),
            flight: None,
        }
    }

    pub fn trigger(&self) -> Trigger {
        self.trigger
    }

    /// Applies a new trigger from the settings. A gesture already in flight keeps
    /// the trigger it started with, so changing the key mid-drag cannot strand a
    /// swallowed button.
    pub fn set_trigger(&mut self, trigger: Trigger) {
        self.trigger = trigger.clamped();
    }

    pub fn is_in_flight(&self) -> bool {
        self.flight.is_some()
    }

    /// True while the wheel is open, which is what the renderer's click-through
    /// and the keyboard hook both ask before doing anything.
    pub fn is_open(&self) -> bool {
        self.flight.map(|f| f.opened).unwrap_or(false)
    }

    /// A button went down somewhere on the desktop.
    pub fn on_press(&mut self, pt: Pt, mods: Mods, button: Button, cx: PressContext) -> OnPress {
        if self.is_open() {
            // The wheel is on screen, so anything else the user presses is aimed
            // at it - the other button to cancel, or the same one twice.
            return OnPress::Take;
        }
        if cx.on_frame || cx.exclusive_foreground {
            return OnPress::Pass;
        }
        let TriggerInput::Mouse(want) = self.trigger.input else {
            // A keyboard trigger leaves every mouse button alone.
            return OnPress::Pass;
        };
        if button != want || !mods.covers(&self.trigger.mods) {
            return OnPress::Pass;
        }
        self.flight = Some(InFlight {
            press: pt,
            since: Instant::now(),
            opened: false,
            swallowed: true,
        });
        OnPress::Take
    }

    /// A key went down. A keyboard trigger starts the same gesture a button
    /// would, centred on the pointer at that moment; everything else passes so
    /// typing and system shortcuts keep working.
    pub fn on_key_down(&mut self, vk: u16, mods: Mods, pt: Pt, cx: PressContext) -> OnPress {
        if self.is_open() || cx.exclusive_foreground {
            return OnPress::Pass;
        }
        let TriggerInput::Key(want) = self.trigger.input else {
            return OnPress::Pass;
        };
        if vk != want || !mods.covers(&self.trigger.mods) {
            return OnPress::Pass;
        }
        self.flight = Some(InFlight {
            press: pt,
            since: Instant::now(),
            opened: false,
            // There is no click to replay: letting go before the hold just
            // returns the key, which the caller swallows and forwards.
            swallowed: false,
        });
        OnPress::Take
    }

    /// The trigger key came back up. Only a keyboard trigger owns this.
    pub fn on_key_up(&mut self, vk: u16, pt: Pt) -> OnRelease {
        let TriggerInput::Key(want) = self.trigger.input else {
            return OnRelease::NotOurs;
        };
        if vk != want {
            return OnRelease::NotOurs;
        }
        self.release_at(pt)
    }

    /// The clock reached the hold while the trigger was still down. Opens once
    /// per gesture and returns the press point the dial is centred on. Nothing
    /// is returned for a short click, which is replayed on release.
    pub fn after_hold(&mut self, hold: Duration) -> Option<Pt> {
        let flight = self.flight?;
        if flight.opened || flight.since.elapsed() < hold {
            return None;
        }
        self.flight = Some(InFlight {
            opened: true,
            ..flight
        });
        Some(flight.press)
    }

    /// The cursor moved. Only a press we took produces anything; before the hold
    /// it is still just a click being watched.
    pub fn on_move(&mut self, pt: Pt) -> OnMove {
        let Some(flight) = self.flight else {
            return OnMove::Nothing;
        };
        if !flight.opened {
            return OnMove::Nothing;
        }
        OnMove::Aim(Offset {
            dx: pt.x - flight.press.x,
            dy: pt.y - flight.press.y,
            distance: flight.press.distance_to(&pt),
        })
    }

    /// The watched button came back up.
    pub fn on_release(&mut self, pt: Pt) -> OnRelease {
        self.release_at(pt)
    }

    /// Ends the gesture at `pt`. Shared by the mouse and keyboard paths so
    /// picking, cancelling and the pre-hold case cannot drift apart.
    fn release_at(&mut self, pt: Pt) -> OnRelease {
        let Some(flight) = self.flight.take() else {
            return OnRelease::NotOurs;
        };
        if flight.opened {
            return OnRelease::Pick(Offset {
                dx: pt.x - flight.press.x,
                dy: pt.y - flight.press.y,
                distance: flight.press.distance_to(&pt),
            });
        }
        if flight.swallowed {
            // Let go before the hold: the user meant a click where they pressed,
            // not a wheel.
            return OnRelease::ReplayClick(flight.press);
        }
        OnRelease::NotOurs
    }

    /// Abandons the gesture without a release - the window was destroyed, the
    /// session is ending, or the watchdog ran out. Returns the press point when
    /// the application is owed its click back.
    pub fn abandon(&mut self) -> Option<Pt> {
        let flight = self.flight.take()?;
        if flight.swallowed && !flight.opened {
            Some(flight.press)
        } else {
            None
        }
    }

    /// `Alt` came up while a gesture was in flight. The wheel stays where it is:
    /// the user is committed, and the release is what picks. But the modifier
    /// that would light up the system menu bar is going away, so the caller
    /// injects the mask key first (design §1, `lolPing`). False when nothing is
    /// in flight, because then the `Alt` was the user's own and must reach the
    /// desktop.
    pub fn needs_mask_key(&self) -> bool {
        self.flight.is_some()
    }

    /// Did this release land back inside the hub, i.e. is it a cancel? The hub's
    /// size belongs to the renderer's [`crate::wheel::Geometry`], so the caller
    /// passes its radius in physical pixels rather than the machine guessing one.
    pub fn hub_cancel(&self, offset: &Offset, hub_physical: f64) -> bool {
        offset.distance < hub_physical
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m() -> Mods {
        Mods::alt()
    }

    fn mach(threshold: i32) -> Machine {
        Machine::new(Trigger {
            threshold,
            ..Default::default()
        })
    }

    #[test]
    fn a_press_without_the_trigger_goes_through_untouched() {
        let mut g = mach(8);
        assert_eq!(
            g.on_press(
                Pt::new(100, 100),
                Mods::default(),
                Button::Left,
                PressContext::default()
            ),
            OnPress::Pass
        );
        assert!(!g.is_in_flight());
    }

    #[test]
    fn the_wrong_button_goes_through_too() {
        let mut g = mach(8);
        assert_eq!(
            g.on_press(Pt::new(0, 0), m(), Button::Right, PressContext::default()),
            OnPress::Pass
        );
    }

    #[test]
    fn carrying_extra_modifiers_does_not_void_the_trigger() {
        let mut g = mach(8);
        let both = Mods { ctrl: true, ..m() };
        assert_eq!(
            g.on_press(Pt::new(0, 0), both, Button::Left, PressContext::default()),
            OnPress::Take
        );
    }

    #[test]
    fn nothing_opens_until_the_trigger_is_held() {
        let mut g = mach(8);
        g.on_press(
            Pt::new(500, 500),
            m(),
            Button::Left,
            PressContext::default(),
        );
        // Travel alone is not a wheel any more: it stays a click until the hold.
        assert_eq!(g.on_move(Pt::new(560, 500)), OnMove::Nothing);
        assert!(!g.is_open());
        assert_eq!(g.after_hold(Duration::ZERO), Some(Pt::new(500, 500)));
        assert!(g.is_open());
        // And afterwards every move just re-aims; the open happens once.
        assert_eq!(g.after_hold(Duration::ZERO), None);
        assert_eq!(
            g.on_move(Pt::new(520, 480)),
            OnMove::Aim(Offset {
                dx: 20,
                dy: -20,
                distance: (800.0f64).sqrt()
            })
        );
    }

    #[test]
    fn a_release_before_the_hold_owes_the_application_its_click() {
        let mut g = mach(8);
        g.on_press(Pt::new(30, 40), m(), Button::Left, PressContext::default());
        g.on_move(Pt::new(32, 41));
        assert_eq!(g.after_hold(Duration::from_millis(350)), None);
        assert_eq!(
            g.on_release(Pt::new(32, 41)),
            OnRelease::ReplayClick(Pt::new(30, 40))
        );
        assert!(!g.is_in_flight());
    }

    #[test]
    fn a_release_after_the_hold_is_picked_up_by_the_wheel() {
        let mut g = mach(8);
        g.on_press(Pt::new(0, 0), m(), Button::Left, PressContext::default());
        assert_eq!(g.after_hold(Duration::ZERO), Some(Pt::new(0, 0)));
        let OnRelease::Pick(o) = g.on_release(Pt::new(60, 0)) else {
            panic!("the release belongs to the wheel");
        };
        assert_eq!(o.dx, 60);
        assert!(!g.is_in_flight());
    }

    #[test]
    fn a_press_on_the_frame_is_never_taken() {
        // Measured in design §6.4/§6.5: with Alt held, a drag on the border
        // resizes (dw +90) and a drag on the caption moves (dx +90 dy +61),
        // exactly as without Alt. Taking either press would steal a gesture the
        // system already owns.
        let mut g = mach(8);
        let cx = PressContext {
            on_frame: true,
            ..Default::default()
        };
        assert_eq!(
            g.on_press(Pt::new(0, 0), m(), Button::Left, cx),
            OnPress::Pass
        );
    }

    #[test]
    fn a_fullscreen_exclusive_foreground_keeps_its_input() {
        let mut g = mach(8);
        let cx = PressContext {
            exclusive_foreground: true,
            ..Default::default()
        };
        assert_eq!(
            g.on_press(Pt::new(0, 0), m(), Button::Left, cx),
            OnPress::Pass
        );
    }

    #[test]
    fn a_failed_replay_is_the_hook_switching_the_gesture_off() {
        // The switch-off lives in `platform-win::hook` because that is where the
        // refused injection is noticed. What the machine guarantees is only that
        // a press it never took leaves nothing in flight to replay.
        let mut g = mach(8);
        assert_eq!(
            g.on_press(
                Pt::new(0, 0),
                Mods::default(),
                Button::Left,
                PressContext::default()
            ),
            OnPress::Pass
        );
        assert_eq!(g.on_release(Pt::new(0, 0)), OnRelease::NotOurs);
    }

    #[test]
    fn while_the_wheel_is_open_any_press_is_swallowed() {
        let mut g = mach(8);
        g.on_press(Pt::new(0, 0), m(), Button::Left, PressContext::default());
        assert_eq!(g.after_hold(Duration::ZERO), Some(Pt::new(0, 0)));
        assert_eq!(
            g.on_press(
                Pt::new(60, 0),
                Mods::default(),
                Button::Right,
                PressContext::default()
            ),
            OnPress::Take
        );
        assert!(g.is_open());
    }

    #[test]
    fn abandoning_owes_the_click_back_only_before_the_wheel_opened() {
        let mut early = mach(8);
        early.on_press(Pt::new(9, 9), m(), Button::Left, PressContext::default());
        assert_eq!(early.abandon(), Some(Pt::new(9, 9)));

        // Moving is aiming, not opening: the clock does that. A press that has
        // travelled but not waited is still a click the desktop owes back.
        let mut late = mach(8);
        late.on_press(Pt::new(9, 9), m(), Button::Left, PressContext::default());
        late.on_move(Pt::new(90, 9));
        assert_eq!(late.abandon(), Some(Pt::new(9, 9)));

        // Once the clock has opened it, nothing is owed to the desktop.
        let mut opened = mach(8);
        opened.on_press(Pt::new(9, 9), m(), Button::Left, PressContext::default());
        assert_eq!(opened.after_hold(Duration::ZERO), Some(Pt::new(9, 9)));
        assert_eq!(opened.abandon(), None);
        assert_eq!(opened.abandon(), None);
    }

    #[test]
    fn alt_let_go_mid_gesture_needs_the_mask_key_but_not_a_cancel() {
        let mut g = mach(8);
        assert!(!g.needs_mask_key());
        g.on_press(Pt::new(0, 0), m(), Button::Left, PressContext::default());
        assert!(g.needs_mask_key());
        g.on_move(Pt::new(50, 0));
        assert!(g.needs_mask_key());
        g.on_release(Pt::new(50, 0));
        assert!(!g.needs_mask_key());
    }

    #[test]
    fn an_unwatched_release_is_none_of_ours() {
        let mut g = mach(8);
        assert_eq!(g.on_release(Pt::new(0, 0)), OnRelease::NotOurs);
        assert_eq!(g.on_move(Pt::new(0, 0)), OnMove::Nothing);
    }

    #[test]
    fn the_threshold_is_clamped_like_every_other_user_number() {
        assert_eq!(
            Trigger {
                threshold: 0,
                ..Default::default()
            }
            .clamped()
            .threshold,
            THRESHOLD_MIN
        );
        assert_eq!(
            Trigger {
                threshold: 9000,
                ..Default::default()
            }
            .clamped()
            .threshold,
            THRESHOLD_MAX
        );
    }

    #[test]
    fn a_release_inside_the_hub_reads_as_a_cancel_to_the_caller() {
        let g = mach(8);
        // Hub 150 * 0.42 = 63 logical px; at 100% that is 63 physical.
        assert!(g.hub_cancel(
            &Offset {
                dx: 10,
                dy: 10,
                distance: 14.0
            },
            63.0
        ));
        assert!(!g.hub_cancel(
            &Offset {
                dx: 60,
                dy: 0,
                distance: 60.0
            },
            40.0
        ));
    }

    #[test]
    fn changing_the_trigger_does_not_strand_a_swallowed_button() {
        let mut g = mach(8);
        g.on_press(Pt::new(0, 0), m(), Button::Left, PressContext::default());
        g.set_trigger(Trigger {
            mods: Mods {
                ctrl: true,
                ..Default::default()
            },
            input: TriggerInput::Mouse(Button::Right),
            threshold: 20,
        });
        // Still in flight from the old trigger, so the release is still ours.
        assert!(g.is_in_flight());
        assert_eq!(
            g.on_release(Pt::new(2, 2)),
            OnRelease::ReplayClick(Pt::new(0, 0))
        );
    }

    fn mach_key(vk: u16, mods: Mods) -> Machine {
        Machine::new(Trigger {
            mods,
            input: TriggerInput::Key(vk),
            threshold: 8,
        })
    }

    #[test]
    fn a_matching_key_opens_the_wheel_on_the_hold() {
        let mut g = mach_key(0x76, Mods::default()); // F7
        assert_eq!(
            g.on_key_down(0x76, Mods::default(), Pt::new(300, 200), PressContext::default()),
            OnPress::Take
        );
        assert_eq!(g.after_hold(Duration::ZERO), Some(Pt::new(300, 200)));
        assert!(g.is_open());
        let OnRelease::Pick(o) = g.on_key_up(0x76, Pt::new(360, 200)) else {
            panic!("the release belongs to the wheel");
        };
        assert_eq!(o.dx, 60);
        assert!(!g.is_in_flight());
    }

    #[test]
    fn other_keys_pass_through() {
        let mut g = mach_key(0x76, Mods::default());
        assert_eq!(
            g.on_key_down(0x75, Mods::default(), Pt::new(0, 0), PressContext::default()),
            OnPress::Pass
        );
        assert!(!g.is_in_flight());
    }

    #[test]
    fn a_key_trigger_leaves_mouse_buttons_alone() {
        let mut g = mach_key(0x76, Mods::alt());
        assert_eq!(
            g.on_press(Pt::new(0, 0), Mods::alt(), Button::Left, PressContext::default()),
            OnPress::Pass
        );
    }

    #[test]
    fn modifiers_must_match_a_key_trigger_too() {
        let mut g = mach_key(0x76, Mods { ctrl: true, shift: true, ..Default::default() });
        assert_eq!(
            g.on_key_down(
                0x76,
                Mods { shift: true, ..Default::default() },
                Pt::new(0, 0),
                PressContext::default()
            ),
            OnPress::Pass
        );
        assert_eq!(
            g.on_key_down(
                0x76,
                Mods { ctrl: true, shift: true, ..Default::default() },
                Pt::new(0, 0),
                PressContext::default()
            ),
            OnPress::Take
        );
    }

    #[test]
    fn releasing_the_key_before_the_hold_is_not_a_replayed_click() {
        let mut g = mach_key(0x76, Mods::default());
        g.on_key_down(0x76, Mods::default(), Pt::new(0, 0), PressContext::default());
        assert_eq!(g.on_key_up(0x76, Pt::new(2, 2)), OnRelease::NotOurs);
    }
}
