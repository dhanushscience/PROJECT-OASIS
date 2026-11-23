//! Port of `SW/tests/Oasis.Architecture.Tests/PadReducerTests.cs`, one test per C# test,
//! same names. These pin the gesture precedence; if one fails, behaviour changed.

use std::time::Duration;

use oasis_core::input::*;
use oasis_core::{ButtonId, ButtonMask, Instant};

fn at(ms: i64) -> Instant {
    Instant::from_millis(ms)
}

fn step(
    state: &PadState,
    evt: PadEvent,
    config: &ArbiterConfig,
    ctx: &ArbiterContext,
) -> Transition {
    reduce(state, evt, config, ctx, &default_claims())
}

fn d() -> ArbiterConfig {
    ArbiterConfig::product_defaults()
}

fn desk() -> ArbiterContext {
    ArbiterContext::desktop()
}

fn yt() -> ArbiterContext {
    ArbiterContext::youtube()
}

use GestureClaimId as G;
use PadEvent::{Disconnected, Down, Tick, Up};

#[test]
fn claims_use_overlay_precedence() {
    let ids: Vec<_> = default_claims().iter().map(|c| c.id()).collect();
    assert_eq!(
        ids,
        vec![
            G::MeetingReminder,
            G::AiSession,
            G::Confirmation,
            G::VoicePtt,
            G::Hold2x,
            G::DoubleClickVolume,
            G::ShowDesktop,
            G::AiHold,
            G::BoundTap
        ]
    );
    let p: Vec<_> = default_claims().iter().map(|c| c.priority()).collect();
    assert!(p.windows(2).all(|w| w[0] >= w[1]));
}

#[test]
fn desktop_tap_executes_on_press_not_release() {
    let down = step(&PadState::Idle, Down(ButtonId::ONE, at(0)), &d(), &desk());
    assert!(down.dispatched(G::BoundTap));
    assert!(matches!(down.next, PadState::Pressed { .. }));

    let up = step(&down.next, Up(ButtonId::ONE, at(50)), &d(), &desk());
    assert_eq!(up.dispatches().count(), 0);
    assert_eq!(up.next, PadState::Idle);
}

/// The C# reducer (a shadow there, not the live path) dispatched the hold-2× button's tap twice
/// outside YouTube: once on press and again on release. A tap must run once.
#[test]
fn a_tap_on_the_hold2x_button_outside_youtube_runs_once() {
    let down = step(&PadState::Idle, Down(ButtonId::TWO, at(0)), &d(), &desk());
    assert!(down.dispatched(G::BoundTap));
    let up = step(&down.next, Up(ButtonId::TWO, at(80)), &d(), &desk());
    assert_eq!(up.dispatches().count(), 0);
}

#[test]
fn release_without_press_does_not_dispatch() {
    let up = step(&PadState::Idle, Up(ButtonId::ONE, at(0)), &d(), &desk());
    assert_eq!(up.dispatches().count(), 0);
    assert_eq!(up.next, PadState::Idle);
}

#[test]
fn ai_on_defers_bound_tap_to_release() {
    let c = ArbiterConfig {
        ai_enabled: true,
        ai_trigger_mask: ButtonMask::TWO,
        ..d()
    };
    let down = step(&PadState::Idle, Down(ButtonId::ONE, at(0)), &c, &desk());
    assert_eq!(down.dispatches().count(), 0);
    let up = step(&down.next, Up(ButtonId::ONE, at(40)), &c, &desk());
    assert!(up.dispatched(G::BoundTap));
}

#[test]
fn youtube_volume_single_tap_runs_bound_action_after_the_window() {
    let c = ArbiterConfig {
        show_overlay_labels: false,
        ..d()
    };
    let down = step(&PadState::Idle, Down(ButtonId::ONE, at(0)), &c, &yt());
    assert_eq!(down.dispatches().count(), 0);

    let up = step(&down.next, Up(ButtonId::ONE, at(40)), &c, &yt());
    assert_eq!(up.dispatches().count(), 0);
    let PadState::AwaitingSecondTap {
        button,
        deadline,
        reveal_only,
    } = up.next
    else {
        panic!("expected AwaitingSecondTap, got {:?}", up.next);
    };
    assert_eq!(button, ButtonId::ONE);
    assert!(!reveal_only);

    let tick = step(&up.next, Tick(deadline), &c, &yt());
    assert!(tick.dispatched(G::BoundTap));
    assert!(!tick.dispatched(G::DoubleClickVolume));
    assert_eq!(tick.next, PadState::Idle);
}

#[test]
fn youtube_volume_double_click_is_volume_not_bound_tap() {
    let t0 = step(&PadState::Idle, Down(ButtonId::ONE, at(0)), &d(), &yt());
    let t1 = step(&t0.next, Up(ButtonId::ONE, at(30)), &d(), &yt());
    let t2 = step(&t1.next, Down(ButtonId::ONE, at(80)), &d(), &yt());
    let t3 = step(&t2.next, Up(ButtonId::ONE, at(110)), &d(), &yt());
    assert!(t3.dispatched(G::DoubleClickVolume));
    assert!(!t3.dispatched(G::BoundTap));
    assert_eq!(t3.next, PadState::Idle);
}

#[test]
fn background_youtube_does_not_arm_hidden_gestures() {
    let down = step(&PadState::Idle, Down(ButtonId::ONE, at(0)), &d(), &desk());
    assert!(down.dispatched(G::BoundTap));
    assert!(matches!(down.next, PadState::Pressed { .. }));
}

#[test]
fn second_volume_tap_stays_volume_if_context_later_flakes() {
    let t0 = step(&PadState::Idle, Down(ButtonId::ONE, at(0)), &d(), &yt());
    let t1 = step(&t0.next, Up(ButtonId::ONE, at(30)), &d(), &yt());
    let t2 = step(&t1.next, Down(ButtonId::ONE, at(80)), &d(), &desk());
    let t3 = step(&t2.next, Up(ButtonId::ONE, at(100)), &d(), &desk());
    assert!(t3.dispatched(G::DoubleClickVolume));
    assert!(!t3.dispatched(G::BoundTap));
}

#[test]
fn hold2x_engage_consumes_release() {
    let down = step(&PadState::Idle, Down(ButtonId::TWO, at(0)), &d(), &yt());
    assert!(down.sets_timer(HOLD2X_TIMER));

    let tick = step(&down.next, Tick(at(250)), &d(), &yt());
    assert!(tick.sends(HOLD2X_ENGAGE_COMMAND));
    assert!(matches!(
        tick.next,
        PadState::Claimed {
            claim: G::Hold2x,
            ..
        }
    ));

    let up = step(&tick.next, Up(ButtonId::TWO, at(400)), &d(), &yt());
    assert!(up.sends(HOLD2X_RESTORE_COMMAND));
    assert_eq!(up.dispatches().count(), 0);
    assert_eq!(up.next, PadState::Idle);
}

#[test]
fn hold2x_short_release_is_bound_tap() {
    let down = step(&PadState::Idle, Down(ButtonId::TWO, at(0)), &d(), &yt());
    let up = step(&down.next, Up(ButtonId::TWO, at(80)), &d(), &yt());
    assert!(up.dispatched(G::BoundTap));
    assert!(!up.dispatched(G::DoubleClickVolume));
}

#[test]
fn ptt_hold_starts_capture_and_release_ends_it() {
    let c = ArbiterConfig {
        voice_enabled: true,
        ..d()
    };
    let down = step(&PadState::Idle, Down(ButtonId::TWO, at(0)), &c, &desk());
    assert!(down.sets_timer(VOICE_PTT_TIMER));
    assert_eq!(down.dispatches().count(), 0);

    let tick = step(&down.next, Tick(at(350)), &c, &desk());
    assert!(tick.has(&ArbiterEffect::StartVoiceCapture));
    assert!(matches!(
        tick.next,
        PadState::Claimed {
            claim: G::VoicePtt,
            ..
        }
    ));

    let up = step(&tick.next, Up(ButtonId::TWO, at(800)), &c, &desk());
    assert!(up.has(&ArbiterEffect::EndVoiceCapture));
    assert!(!up.dispatched(G::BoundTap));
}

#[test]
fn another_button_during_a_talk_hold_does_not_lose_the_release() {
    let c = ArbiterConfig { voice_enabled: true, push_to_talk_mask: ButtonMask::ONE, ..d() };
    let down = step(&PadState::Idle, Down(ButtonId::ONE, at(0)), &c, &desk());
    let talking = step(&down.next, Tick(at(500)), &c, &desk());
    assert!(talking.has(&ArbiterEffect::StartVoiceCapture));
    // a brush of another button while talking: neither it nor its release may end or forget the hold
    let brush = step(&talking.next, Down(ButtonId::TWO, at(900)), &c, &desk());
    assert!(!brush.dispatched(G::BoundTap), "no action runs while talking");
    let brush_up = step(&brush.next, Up(ButtonId::TWO, at(950)), &c, &desk());
    let up = step(&brush_up.next, Up(ButtonId::ONE, at(2000)), &c, &desk());
    assert!(up.has(&ArbiterEffect::EndVoiceCapture), "letting go of the talk button must end the recording");
}

#[test]
fn ptt_short_release_is_a_tap() {
    let c = ArbiterConfig {
        voice_enabled: true,
        ..d()
    };
    let down = step(&PadState::Idle, Down(ButtonId::TWO, at(0)), &c, &desk());
    let up = step(&down.next, Up(ButtonId::TWO, at(80)), &c, &desk());
    // The tap waits one double-click window, in case a second tap locks the mic on.
    assert!(!up.dispatched(G::BoundTap));
    let late = step(&up.next, Tick(at(80 + 300)), &c, &desk());
    assert!(late.dispatched(G::BoundTap));
    assert_eq!(late.next, PadState::Idle);
    assert!(!late.has(&ArbiterEffect::StartVoiceCapture));
}

#[test]
fn ptt_double_click_locks_the_mic_on_until_the_next_press() {
    let c = ArbiterConfig {
        voice_enabled: true,
        ..d()
    };
    let b = ButtonId::TWO;
    let down = step(&PadState::Idle, Down(b, at(0)), &c, &desk());
    let up = step(&down.next, Up(b, at(80)), &c, &desk());
    let second = step(&up.next, Down(b, at(200)), &c, &desk());
    assert!(second.has(&ArbiterEffect::StartVoiceCapture));
    assert!(!second.dispatched(G::BoundTap), "a double-click is not a tap");
    assert_eq!(second.next, PadState::VoiceLocked);

    // letting go keeps it recording; no timer ends it
    let release = step(&second.next, Up(b, at(260)), &c, &desk());
    assert_eq!(release.next, PadState::VoiceLocked);
    assert!(!release.has(&ArbiterEffect::EndVoiceCapture));
    let later = step(&release.next, Tick(at(20_000)), &c, &desk());
    assert_eq!(later.next, PadState::VoiceLocked);

    // the next press ends it, and its own release does nothing more
    let end = step(&later.next, Down(b, at(30_000)), &c, &desk());
    assert!(end.has(&ArbiterEffect::EndVoiceCapture));
    let last = step(&end.next, Up(b, at(30_100)), &c, &desk());
    assert_eq!(last.next, PadState::Idle);
    assert!(!last.dispatched(G::BoundTap));
}

#[test]
fn button_two_cancels_a_locked_recording() {
    let c = ArbiterConfig {
        voice_enabled: true,
        push_to_talk_mask: ButtonMask::ONE,
        ..d()
    };
    let cancel = step(&PadState::VoiceLocked, Down(ButtonId::TWO, at(0)), &c, &desk());
    assert!(cancel.has(&ArbiterEffect::CancelVoiceCapture));
    assert!(!cancel.has(&ArbiterEffect::EndVoiceCapture));
    let up = step(&cancel.next, Up(ButtonId::TWO, at(90)), &c, &desk());
    assert_eq!(up.next, PadState::Idle);
    assert!(!up.dispatched(G::BoundTap));
    assert!(up.effects.is_empty());
}

#[test]
fn ptt_requires_exact_mask() {
    let c = ArbiterConfig {
        voice_enabled: true,
        push_to_talk_mask: ButtonMask::ONE,
        ..d()
    };
    let other = step(&PadState::Idle, Down(ButtonId::TWO, at(0)), &c, &desk());
    assert!(!other.sets_timer(VOICE_PTT_TIMER));

    let first = step(&PadState::Idle, Down(ButtonId::ONE, at(0)), &c, &desk());
    assert!(first.sets_timer(VOICE_PTT_TIMER));

    let extra = step(&first.next, Down(ButtonId::TWO, at(40)), &c, &desk());
    assert!(extra.cancels_timer(VOICE_PTT_TIMER));
    let PadState::Pressed { arm, .. } = extra.next else {
        panic!("{:?}", extra.next)
    };
    assert!(arm.voice_ptt_deadline.is_none());
}

#[test]
fn user_pad_hold2x_is_button_three() {
    let c = ArbiterConfig::user_pad();
    let down = step(&PadState::Idle, Down(ButtonId::THREE, at(0)), &c, &yt());
    assert!(down.sets_timer(HOLD2X_TIMER));
    let tick = step(&down.next, Tick(at(250)), &c, &yt());
    assert!(tick.sends(HOLD2X_ENGAGE_COMMAND));
}

#[test]
fn shared_ptt_and_volume_short_press_arms_volume_window() {
    let c = ArbiterConfig::user_pad();
    let down = step(&PadState::Idle, Down(ButtonId::ONE, at(0)), &c, &yt());
    assert!(down.sets_timer(VOICE_PTT_TIMER));
    assert_eq!(down.dispatches().count(), 0);

    let up = step(&down.next, Up(ButtonId::ONE, at(80)), &c, &yt());
    assert!(matches!(up.next, PadState::AwaitingSecondTap { .. }));
    assert!(!up.dispatched(G::BoundTap));
}

#[test]
fn flush_other_button_runs_the_pending_action_first() {
    let c = ArbiterConfig {
        show_overlay_labels: false,
        ..d()
    };
    let t0 = step(&PadState::Idle, Down(ButtonId::ONE, at(0)), &c, &yt());
    let t1 = step(&t0.next, Up(ButtonId::ONE, at(40)), &c, &yt());
    let t2 = step(&t1.next, Down(ButtonId::THREE, at(80)), &c, &yt());
    assert!(t2.dispatched(G::BoundTap));
    assert!(t2
        .dispatches()
        .any(|(b, c)| b == ButtonId::ONE && c == G::BoundTap));
    assert!(matches!(t2.next, PadState::Pressed { .. }));
}

#[test]
fn meeting_consumes_the_press() {
    let ctx = ArbiterContext {
        meeting_reminder_active: true,
        ..desk()
    };
    let down = step(&PadState::Idle, Down(ButtonId::ONE, at(0)), &d(), &ctx);
    assert!(down.dispatched(G::MeetingReminder));
    assert!(!down.dispatched(G::BoundTap));
    assert!(matches!(
        down.next,
        PadState::Claimed {
            claim: G::MeetingReminder,
            ..
        }
    ));
}

#[test]
fn confirmation_records_press_but_does_not_run_the_bound_action() {
    let ctx = ArbiterContext {
        confirmation_pending: true,
        ..desk()
    };
    let down = step(&PadState::Idle, Down(ButtonId::ONE, at(0)), &d(), &ctx);
    assert_eq!(down.dispatches().count(), 0);
    assert!(matches!(down.next, PadState::Pressed { .. }));

    let up = step(&down.next, Up(ButtonId::ONE, at(40)), &d(), &ctx);
    assert!(up.dispatched(G::Confirmation));
    assert!(!up.dispatched(G::BoundTap));
}

#[test]
fn integrity_block_still_records_press_but_skips_bound_tap() {
    let ctx = ArbiterContext {
        execution_allowed: false,
        ..desk()
    };
    let down = step(&PadState::Idle, Down(ButtonId::ONE, at(0)), &d(), &ctx);
    assert!(matches!(down.next, PadState::Pressed { .. }));
    assert!(!down.dispatched(G::BoundTap));
}

#[test]
fn reveal_only_expiry_does_not_run_the_bound_action() {
    let c = ArbiterConfig {
        show_overlay_labels: true,
        ..d()
    };
    let ctx = ArbiterContext {
        labels_on_screen: false,
        ..yt()
    };
    let down = step(&PadState::Idle, Down(ButtonId::ONE, at(0)), &c, &ctx);
    let up = step(&down.next, Up(ButtonId::ONE, at(40)), &c, &ctx);
    assert!(up
        .effects
        .iter()
        .any(|e| matches!(e, ArbiterEffect::ShowPeek(_))));
    let PadState::AwaitingSecondTap {
        reveal_only,
        deadline,
        ..
    } = up.next
    else {
        panic!("{:?}", up.next)
    };
    assert!(reveal_only);

    let tick = step(&up.next, Tick(deadline), &c, &ctx);
    assert!(!tick.dispatched(G::BoundTap));
    assert_eq!(tick.next, PadState::Idle);
}

#[test]
fn ai_hold_tick_claims_ai_and_consumes_the_release() {
    let c = ArbiterConfig {
        ai_enabled: true,
        ai_trigger_mask: ButtonMask::TWO,
        ..d()
    };
    let down = step(&PadState::Idle, Down(ButtonId::TWO, at(0)), &c, &desk());
    assert!(down.sets_timer(AI_HOLD_TIMER));

    let tick = step(&down.next, Tick(at(5000)), &c, &desk());
    assert!(tick.dispatched(G::AiHold));
    assert!(matches!(
        tick.next,
        PadState::Claimed {
            claim: G::AiHold,
            ..
        }
    ));

    let up = step(&tick.next, Up(ButtonId::TWO, at(5010)), &c, &desk());
    assert!(!up.dispatched(G::BoundTap));
}

#[test]
fn voice_on_same_button_as_hold2x_wins_at_press() {
    let c = ArbiterConfig {
        voice_enabled: true,
        ..d()
    };
    let down = step(&PadState::Idle, Down(ButtonId::TWO, at(0)), &c, &yt());
    assert!(down.sets_timer(VOICE_PTT_TIMER));
    assert!(!down.sets_timer(HOLD2X_TIMER));
}

#[test]
fn disconnected_ends_voice_and_returns_to_idle() {
    let c = ArbiterConfig {
        voice_enabled: true,
        ..d()
    };
    let down = step(&PadState::Idle, Down(ButtonId::TWO, at(0)), &c, &desk());
    let tick = step(&down.next, Tick(at(350)), &c, &desk());
    let cut = step(&tick.next, Disconnected, &c, &desk());
    assert_eq!(cut.next, PadState::Idle);
    assert!(cut.has(&ArbiterEffect::EndVoiceCapture));
}

#[test]
fn volume_window_is_the_setting_with_no_floor() {
    // The volume buttons are also PREV / JUMP: a 700 ms floor turned two seeks into volume.
    for labels in [false, true] {
        let c = ArbiterConfig {
            show_overlay_labels: labels,
            ..d()
        };
        let down = step(&PadState::Idle, Down(ButtonId::ONE, at(0)), &c, &yt());
        let up = step(&down.next, Up(ButtonId::ONE, at(40)), &c, &yt());
        assert_eq!(
            up.timer_delay(DOUBLE_CLICK_TIMER),
            Some(Duration::from_millis(300))
        );
    }
}

#[test]
fn second_seek_after_the_window_is_a_second_seek() {
    let c = ArbiterConfig {
        show_overlay_labels: false,
        ..d()
    };
    let down = step(&PadState::Idle, Down(ButtonId::THREE, at(0)), &c, &yt());
    let up = step(&down.next, Up(ButtonId::THREE, at(40)), &c, &yt());
    let expired = step(&up.next, Tick(at(360)), &c, &yt());
    assert!(expired.dispatched(G::BoundTap));

    let down2 = step(&expired.next, Down(ButtonId::THREE, at(420)), &c, &yt());
    let up2 = step(&down2.next, Up(ButtonId::THREE, at(460)), &c, &yt());
    assert!(!up2.dispatched(G::DoubleClickVolume));
}

#[test]
fn voice_hold_on_a_volume_button_still_waits_700ms() {
    assert_eq!(d().push_to_talk_delay(true), Duration::from_millis(700));
}

#[test]
fn context_does_not_gate_on_process_name() {
    // The C# test reflects over properties; here the struct is the contract: it has no
    // process-name field, so the reducer cannot gate on one.
    let down = step(&PadState::Idle, Down(ButtonId::ONE, at(0)), &d(), &desk());
    assert!(down.dispatched(G::BoundTap));
}

// Show-desktop hold: BTN 3 by default, 800 ms.

#[test]
fn show_desktop_hold_fires_after_the_threshold_and_consumes_release() {
    let down = step(&PadState::Idle, Down(ButtonId::THREE, at(0)), &d(), &desk());
    assert_eq!(
        down.dispatches().count(),
        0,
        "the tap cannot run until we know it was not a hold"
    );
    assert_eq!(
        down.timer_delay(SHOW_DESKTOP_TIMER),
        Some(Duration::from_millis(800))
    );

    let early = step(&down.next, Tick(at(799)), &d(), &desk());
    assert_eq!(early.dispatches().count(), 0);

    let tick = step(&early.next, Tick(at(800)), &d(), &desk());
    assert!(tick.dispatched(G::ShowDesktop));
    assert!(matches!(
        tick.next,
        PadState::Claimed {
            claim: G::ShowDesktop,
            ..
        }
    ));

    let up = step(&tick.next, Up(ButtonId::THREE, at(1200)), &d(), &desk());
    assert_eq!(up.dispatches().count(), 0);
    assert_eq!(up.next, PadState::Idle);
}

#[test]
fn show_desktop_button_tap_runs_bound_action_on_release() {
    let down = step(&PadState::Idle, Down(ButtonId::THREE, at(0)), &d(), &desk());
    let up = step(&down.next, Up(ButtonId::THREE, at(150)), &d(), &desk());
    assert!(up.dispatched(G::BoundTap));
    assert!(!up.dispatched(G::ShowDesktop));
    assert!(up.cancels_timer(SHOW_DESKTOP_TIMER));
    assert_eq!(up.next, PadState::Idle);
}

#[test]
fn other_buttons_still_tap_on_press() {
    let down = step(&PadState::Idle, Down(ButtonId::TWO, at(0)), &d(), &desk());
    assert!(down.dispatched(G::BoundTap));
    assert!(!down.sets_timer(SHOW_DESKTOP_TIMER));
}

#[test]
fn show_desktop_off_keeps_tap_on_press() {
    let c = ArbiterConfig {
        show_desktop: None,
        ..d()
    };
    let down = step(&PadState::Idle, Down(ButtonId::THREE, at(0)), &c, &desk());
    assert!(down.dispatched(G::BoundTap));
}

#[test]
fn hold2x_on_the_same_button_wins_while_youtube_plays() {
    let c = ArbiterConfig {
        hold2x: Some(ButtonId::THREE),
        volume_down: None,
        ..d()
    };
    let down = step(&PadState::Idle, Down(ButtonId::THREE, at(0)), &c, &yt());
    assert!(down.sets_timer(HOLD2X_TIMER));
    assert!(!down.sets_timer(SHOW_DESKTOP_TIMER));
    let tick = step(&down.next, Tick(at(900)), &c, &yt());
    assert!(!tick.dispatched(G::ShowDesktop));
}

#[test]
fn hold2x_elsewhere_leaves_show_desktop_armed_outside_youtube() {
    let c = ArbiterConfig {
        hold2x: Some(ButtonId::THREE),
        ..d()
    };
    let down = step(&PadState::Idle, Down(ButtonId::THREE, at(0)), &c, &desk());
    let tick = step(&down.next, Tick(at(800)), &c, &desk());
    assert!(tick.dispatched(G::ShowDesktop));
}

#[test]
fn ai_hold_on_the_same_button_keeps_the_press() {
    let c = ArbiterConfig {
        ai_enabled: true,
        ai_trigger_mask: ButtonMask::THREE,
        ..d()
    };
    let down = step(&PadState::Idle, Down(ButtonId::THREE, at(0)), &c, &desk());
    assert!(down.sets_timer(AI_HOLD_TIMER));
    assert!(!down.sets_timer(SHOW_DESKTOP_TIMER));
}

#[test]
fn voice_ptt_on_the_same_button_keeps_the_press() {
    let c = ArbiterConfig {
        voice_enabled: true,
        push_to_talk_mask: ButtonMask::THREE,
        ..d()
    };
    let down = step(&PadState::Idle, Down(ButtonId::THREE, at(0)), &c, &desk());
    assert!(down.sets_timer(VOICE_PTT_TIMER));
    assert!(!down.sets_timer(SHOW_DESKTOP_TIMER));
}

#[test]
fn second_button_cancels_show_desktop_but_first_tap_still_resolves() {
    let down3 = step(&PadState::Idle, Down(ButtonId::THREE, at(0)), &d(), &desk());
    let down1 = step(&down3.next, Down(ButtonId::ONE, at(100)), &d(), &desk());
    assert!(down1.cancels_timer(SHOW_DESKTOP_TIMER));

    let tick = step(&down1.next, Tick(at(900)), &d(), &desk());
    assert!(!tick.dispatched(G::ShowDesktop));

    let up1 = step(&tick.next, Up(ButtonId::ONE, at(950)), &d(), &desk());
    let up3 = step(&up1.next, Up(ButtonId::THREE, at(1000)), &d(), &desk());
    assert!(up3
        .dispatches()
        .any(|(b, c)| b == ButtonId::THREE && c == G::BoundTap));
}

#[test]
fn show_desktop_does_not_fire_when_execution_is_blocked() {
    let blocked = ArbiterContext {
        execution_allowed: false,
        ..desk()
    };
    let down = step(
        &PadState::Idle,
        Down(ButtonId::THREE, at(0)),
        &d(),
        &blocked,
    );
    let tick = step(&down.next, Tick(at(800)), &d(), &blocked);
    assert_eq!(tick.dispatches().count(), 0);
}

#[test]
fn show_desktop_hold_is_clamped_to_a_reachable_range() {
    let lo = ArbiterConfig {
        show_desktop_hold: Duration::from_millis(50),
        ..d()
    };
    assert_eq!(
        lo.show_desktop_delay(),
        Duration::from_millis(ArbiterConfig::SHOW_DESKTOP_HOLD_MIN_MS)
    );
    let hi = ArbiterConfig {
        show_desktop_hold: Duration::from_secs(30),
        ..d()
    };
    assert_eq!(
        hi.show_desktop_delay(),
        Duration::from_millis(ArbiterConfig::SHOW_DESKTOP_HOLD_MAX_MS)
    );
}
