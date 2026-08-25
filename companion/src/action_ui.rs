use crate::action::{
    ActionKind, ActionSettings, MAX_SHOCK_DURATION, MAX_SHOCK_INTENSITY, MAX_VIBRATE_DURATION,
    MAX_VIBRATE_STRENGTH, MIN_SHOCK_DURATION, MIN_SHOCK_INTENSITY, MIN_VIBRATE_DURATION,
    MIN_VIBRATE_STRENGTH, ShockActionSettings, ShockMode, VibrateActionSettings, VibrateMode,
};
use egui::{Color32, Ui};
use std::ops::RangeInclusive;

pub fn draw_action_editor(ui: &mut Ui, kind: ActionKind, settings: &mut ActionSettings) {
    match (kind, settings) {
        (ActionKind::Shock, settings) => draw_shock_settings_editor(ui, &mut settings.shock),
        (ActionKind::Vibrate, settings) => {
            draw_vibrate_settings_editor(ui, &mut settings.vibrate)
        }
    }
}

pub fn draw_vibrate_settings_editor(ui: &mut Ui, vibrate: &mut VibrateActionSettings) {
    ui.heading("Vibrate mode");
    ui.horizontal(|ui| {
        ui.label("Mode");
        egui::ComboBox::from_id_salt("vibrate-mode")
            .selected_text(vibrate.mode.label())
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut vibrate.mode, VibrateMode::Interval, "Interval");
                ui.selectable_value(&mut vibrate.mode, VibrateMode::Fixed, "Fixed");
            });
    });
    ui.add_space(4.0);
    match vibrate.mode {
        VibrateMode::Interval => {
            slider_input(
                ui,
                "Minimum strength",
                &mut vibrate.interval.minimum_strength,
                MIN_VIBRATE_STRENGTH..=MAX_VIBRATE_STRENGTH,
                1.0,
                "/20",
            );
            slider_input(
                ui,
                "Maximum strength",
                &mut vibrate.interval.maximum_strength,
                MIN_VIBRATE_STRENGTH..=MAX_VIBRATE_STRENGTH,
                1.0,
                "/20",
            );
            slider_input(
                ui,
                "Minimum duration",
                &mut vibrate.interval.minimum_duration_seconds,
                MIN_VIBRATE_DURATION..=MAX_VIBRATE_DURATION,
                1.0,
                " s",
            );
            slider_input(
                ui,
                "Maximum duration",
                &mut vibrate.interval.maximum_duration_seconds,
                MIN_VIBRATE_DURATION..=MAX_VIBRATE_DURATION,
                1.0,
                " s",
            );
        }
        VibrateMode::Fixed => {
            slider_input(
                ui,
                "Strength",
                &mut vibrate.fixed.strength,
                MIN_VIBRATE_STRENGTH..=MAX_VIBRATE_STRENGTH,
                1.0,
                "/20",
            );
            slider_input(
                ui,
                "Duration",
                &mut vibrate.fixed.duration_seconds,
                MIN_VIBRATE_DURATION..=MAX_VIBRATE_DURATION,
                1.0,
                " s",
            );
        }
    }
    if vibrate.interval.minimum_strength > vibrate.interval.maximum_strength {
        vibrate.interval.maximum_strength = vibrate.interval.minimum_strength;
    }
    if vibrate.interval.minimum_duration_seconds > vibrate.interval.maximum_duration_seconds {
        vibrate.interval.maximum_duration_seconds = vibrate.interval.minimum_duration_seconds;
    }
}

pub fn draw_shock_settings_editor(ui: &mut Ui, shock: &mut ShockActionSettings) {
    ui.heading("Shock mode");
    ui.horizontal(|ui| {
        ui.label("Mode");
        egui::ComboBox::from_id_salt("shock-mode")
            .selected_text(shock.mode.label())
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut shock.mode, ShockMode::Interval, "Interval");
                ui.selectable_value(&mut shock.mode, ShockMode::Fixed, "Fixed");
            });
    });
    ui.add_space(4.0);
    match shock.mode {
        ShockMode::Interval => {
            slider_input(
                ui,
                "Minimum intensity",
                &mut shock.interval.minimum_intensity,
                MIN_SHOCK_INTENSITY..=MAX_SHOCK_INTENSITY,
                1.0,
                "%",
            );
            slider_input(
                ui,
                "Maximum intensity",
                &mut shock.interval.maximum_intensity,
                MIN_SHOCK_INTENSITY..=MAX_SHOCK_INTENSITY,
                1.0,
                "%",
            );
            slider_input(
                ui,
                "Minimum duration",
                &mut shock.interval.minimum_duration_seconds,
                MIN_SHOCK_DURATION..=MAX_SHOCK_DURATION,
                0.1,
                " s",
            );
            slider_input(
                ui,
                "Maximum duration",
                &mut shock.interval.maximum_duration_seconds,
                MIN_SHOCK_DURATION..=MAX_SHOCK_DURATION,
                0.1,
                " s",
            );
        }
        ShockMode::Fixed => {
            slider_input(
                ui,
                "Intensity",
                &mut shock.fixed.intensity,
                MIN_SHOCK_INTENSITY..=MAX_SHOCK_INTENSITY,
                1.0,
                "%",
            );
            slider_input(
                ui,
                "Duration",
                &mut shock.fixed.duration_seconds,
                MIN_SHOCK_DURATION..=MAX_SHOCK_DURATION,
                0.1,
                " s",
            );
        }
    }
    if shock.interval.minimum_intensity > shock.interval.maximum_intensity {
        shock.interval.maximum_intensity = shock.interval.minimum_intensity;
    }
    if shock.interval.minimum_duration_seconds > shock.interval.maximum_duration_seconds {
        shock.interval.maximum_duration_seconds = shock.interval.minimum_duration_seconds;
    }
}

fn input_background() -> Color32 {
    Color32::from_rgb(38, 38, 42)
}
fn slider_input<T: egui::emath::Numeric>(
    ui: &mut Ui,
    label: &str,
    value: &mut T,
    range: RangeInclusive<T>,
    step: f64,
    suffix: &str,
) {
    ui.label(label);
    ui.scope(|ui| {
        ui.spacing_mut().slider_width = ui.available_width() * 0.8;
        let visuals = ui.visuals_mut();
        visuals.widgets.inactive.bg_fill = input_background();
        visuals.widgets.hovered.bg_fill = input_background();
        visuals.widgets.active.bg_fill = input_background();
        ui.add(egui::Slider::new(value, range).step_by(step).suffix(suffix));
    });
}
