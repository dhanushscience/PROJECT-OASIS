//! Button down: which gestures a press arms, and taps that run on press.

use super::*;

// ---------------------------------------------------------------- press

pub(super) fn on_down(
    mut state: PadState,
    button: ButtonId,
    at: Instant,
    config: &ArbiterConfig,
    context: &ArbiterContext,
    claims: Claims<'_>,
) -> Transition {
    let mut effects = Vec::new();

    // While talking, only letting go of the talk button counts. Another press used to replace this
    // state, so the release never ended the recording (it stayed open until the next hold).
    if matches!(&state, PadState::Claimed { claim: GestureClaimId::VoicePtt, .. })
        || matches!(&state, PadState::Pressed { arm, .. } if arm.voice_capturing)
    {
        return Transition::to(state);
    }

    if state == PadState::VoiceLocked {
        // The next press of the talk button ends the recording, like letting go of a hold.
        if !config.push_to_talk_mask.contains(button) {
            // Button 2 throws the recording away (the cross on the on-screen pill).
            if button == ButtonId::TWO {
                effects.push(ArbiterEffect::CancelVoiceCapture);
                // Its release must do nothing: a press that already "dispatched" resolves to Idle silently.
                return Transition::with(
                    PadState::Pressed {
                        held: button.mask(),
                        since: at,
                        last: button,
                        arm: PressArm {
                            dispatched_on_press: true,
                            ..PressArm::default()
                        },
                    },
                    effects,
                );
            }
            return Transition::to(state);
        }
        effects.push(ArbiterEffect::EndVoiceCapture);
        return Transition::with(PadState::claimed(GestureClaimId::VoicePtt, button.mask()), effects);
    }

    if allows(claims, GestureClaimId::MeetingReminder, &state, context)
        && context.meeting_reminder_active
    {
        effects.push(dispatch(button, GestureClaimId::MeetingReminder));
        return Transition::with(
            PadState::claimed(GestureClaimId::MeetingReminder, button.mask()),
            effects,
        );
    }

    if let PadState::AwaitingSecondTap {
        button: pending,
        reveal_only,
        ..
    } = state
    {
        if pending == button && is_ptt_lock_button(button, config) && !is_hidden_volume(button, config, context, claims, &state) {
            // Second tap of a double-click on the talk button: start listening and stay on.
            effects.push(ArbiterEffect::CancelTimer(DOUBLE_CLICK_TIMER));
            effects.push(ArbiterEffect::StartVoiceCapture);
            return Transition::with(PadState::VoiceLocked, effects);
        }
        if pending == button {
            // Second tap of a double-click: the release decides it is volume.
            return Transition::to(PadState::Pressed {
                held: button.mask(),
                since: at,
                last: button,
                arm: PressArm {
                    completing_volume: true,
                    volume_candidate: true,
                    ..PressArm::default()
                },
            });
        }

        // Another button flushes the pending tap first.
        if !reveal_only {
            try_dispatch_bound_tap(pending, context, claims, &state, &mut effects);
        }
        effects.push(ArbiterEffect::CancelTimer(DOUBLE_CLICK_TIMER));
        state = PadState::Idle;
    }

    let (previous_held, previous_arm) = match &state {
        PadState::Pressed { held, arm, .. } => (*held, *arm),
        _ => (ButtonMask::NONE, PressArm::default()),
    };
    let held_now = previous_held.with(button);
    let pressed = |arm: PressArm| PadState::Pressed {
        held: held_now,
        since: at,
        last: button,
        arm,
    };

    if context.ai_session_active && allows(claims, GestureClaimId::AiSession, &state, context) {
        return Transition::to(PadState::claimed(GestureClaimId::AiSession, held_now));
    }

    if context.confirmation_pending && allows(claims, GestureClaimId::Confirmation, &state, context)
    {
        return Transition::to(pressed(PressArm::default()));
    }


    let volume_hidden = is_hidden_volume(button, config, context, claims, &state);
    let mut arm = PressArm {
        completing_volume: false,
        ..previous_arm
    };

    if previous_arm.voice_ptt_deadline.is_some() && held_now != config.push_to_talk_mask {
        effects.push(ArbiterEffect::CancelTimer(VOICE_PTT_TIMER));
        arm.voice_ptt_deadline = None;
    }

    // A second button turns the desktop hold into a chord: stop the timer, but the first
    // button's tap still resolves on its release.
    if previous_arm.show_desktop_deadline.is_some() {
        effects.push(ArbiterEffect::CancelTimer(SHOW_DESKTOP_TIMER));
        arm.show_desktop_deadline = None;
    }

    let ptt_matched = config.voice_enabled
        && !config.push_to_talk_mask.is_empty()
        && held_now == config.push_to_talk_mask
        && allows(claims, GestureClaimId::VoicePtt, &state, context);

    if ptt_matched {
        let delay = config.push_to_talk_delay(volume_hidden);
        arm.voice_ptt_deadline = Some(at.plus(delay));
        effects.push(set_timer(VOICE_PTT_TIMER, delay));
        if !volume_hidden {
            // PTT-only press returns before hold-2× / volume / bound tap.
            return Transition::with(pressed(arm), effects);
        }
    }

    if config.is_hold2x_button(button)
        && allows(claims, GestureClaimId::Hold2x, &state, context)
        && context.youtube_gestures_active
    {
        let delay = config.hold2x_threshold;
        arm.hold2x_deadline = Some(at.plus(delay));
        arm.hold2x_tracked = true;
        effects.push(set_timer(HOLD2X_TIMER, delay));
        arm_ai_hold(
            &mut arm,
            held_now,
            at,
            config,
            context,
            claims,
            &state,
            &mut effects,
        );
        return Transition::with(pressed(arm), effects);
    }

    if arms_show_desktop(button, held_now, &arm, config, context, claims, &state) {
        let delay = config.show_desktop_delay();
        arm.show_desktop_deadline = Some(at.plus(delay));
        arm.show_desktop_tracked = true;
        effects.push(set_timer(SHOW_DESKTOP_TIMER, delay));
    }

    if volume_hidden {
        arm.volume_candidate = true;
        arm_ai_hold(
            &mut arm,
            held_now,
            at,
            config,
            context,
            claims,
            &state,
            &mut effects,
        );
        return Transition::with(pressed(arm), effects);
    }

    if arm_ai_hold(
        &mut arm,
        held_now,
        at,
        config,
        context,
        claims,
        &state,
        &mut effects,
    ) {
        return Transition::with(pressed(arm), effects);
    }

    // A desktop-hold button cannot know it was a tap until release.
    if config.ai_owns_press_path() || arm.show_desktop_deadline.is_some() {
        return Transition::with(pressed(arm), effects);
    }

    if context.execution_allowed && allows(claims, GestureClaimId::BoundTap, &state, context) {
        effects.push(dispatch(button, GestureClaimId::BoundTap));
        arm.dispatched_on_press = true;
    }

    Transition::with(pressed(arm), effects)
}

/// Arms the AI-hold timer when the AI trigger chord is held. Returns whether it armed.
#[allow(clippy::too_many_arguments)]
fn arm_ai_hold(
    arm: &mut PressArm,
    held_now: ButtonMask,
    at: Instant,
    config: &ArbiterConfig,
    context: &ArbiterContext,
    claims: Claims<'_>,
    state: &PadState,
    effects: &mut Vec<ArbiterEffect>,
) -> bool {
    if config.ai_trigger_held(held_now) && allows(claims, GestureClaimId::AiHold, state, context) {
        arm.ai_hold_deadline = Some(at.plus(config.ai_hold_duration));
        effects.push(set_timer(AI_HOLD_TIMER, config.ai_hold_duration));
        true
    } else {
        false
    }
}
