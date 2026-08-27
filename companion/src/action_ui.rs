use crate::action::{
    MAX_VIBRATE_DURATION, MAX_VIBRATE_STRENGTH, MIN_VIBRATE_DURATION, MIN_VIBRATE_STRENGTH,
    VibrateActionSettings, VibrateMode,
};
use crate::theme::ACCENT;
use egui::{Color32, Ui};
use std::ops::RangeInclusive;

pub fn draw_vibrate_settings_editor(ui: &mut Ui, vibrate: &mut VibrateActionSettings) {
    ui.horizontal(|ui| {
        crate::theme::label_nudged_down(ui, "Mode", 10.0);
        egui::ComboBox::from_id_salt("vibrate-mode")
            .selected_text(vibrate.mode.label())
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut vibrate.mode, VibrateMode::Interval, "Random");
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
                "",
            );
            slider_input(
                ui,
                "Maximum strength",
                &mut vibrate.interval.maximum_strength,
                MIN_VIBRATE_STRENGTH..=MAX_VIBRATE_STRENGTH,
                1.0,
                "",
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
                "",
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

fn input_background() -> Color32 {
    Color32::from_rgb(42, 30, 40)
}
fn slider_input(
    ui: &mut Ui,
    label: &str,
    value: &mut f32,
    range: RangeInclusive<f32>,
    step: f64,
    unit: &str,
) {
    ui.label(label);
    ui.horizontal(|ui| {
        ui.scope(|ui| {
            ui.spacing_mut().slider_width = ui.available_width() * 0.62;
            let visuals = ui.visuals_mut();
            visuals.widgets.inactive.bg_fill = input_background();
            visuals.widgets.hovered.bg_fill = input_background();
            visuals.widgets.active.bg_fill = input_background();
            visuals.selection.bg_fill = ACCENT;
            ui.add(
                egui::Slider::new(value, range.clone())
                    .step_by(step)
                    .show_value(false)
                    .trailing_fill(true),
            );
        });
        ui.weak(format!("{:.2}{unit} / {:.0}{unit}", *value, range.end()));
    });
    ui.add_space(4.0);
}
