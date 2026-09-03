use rand::Rng;
use std::fmt;

pub const MIN_VIBRATE_STRENGTH: f32 = 0.0;
pub const MAX_VIBRATE_STRENGTH: f32 = 20.0;
pub const MIN_VIBRATE_DURATION: f32 = 1.0;
pub const MAX_VIBRATE_DURATION: f32 = 30.0;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum VibrateMode {
    #[default]
    Interval,
    Fixed,
}
impl VibrateMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Interval => "Random",
            Self::Fixed => "Fixed",
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct VibrateIntervalSettings {
    pub minimum_strength: f32,
    pub maximum_strength: f32,
    pub minimum_duration_seconds: f32,
    pub maximum_duration_seconds: f32,
}
#[derive(Clone, Debug, PartialEq)]
pub struct VibrateFixedSettings {
    pub strength: f32,
    pub duration_seconds: f32,
}
#[derive(Clone, Debug, PartialEq)]
pub struct VibrateActionSettings {
    pub mode: VibrateMode,
    pub interval: VibrateIntervalSettings,
    pub fixed: VibrateFixedSettings,
}
impl Default for VibrateActionSettings {
    fn default() -> Self {
        Self {
            mode: VibrateMode::default(),
            interval: VibrateIntervalSettings {
                minimum_strength: MIN_VIBRATE_STRENGTH,
                maximum_strength: MIN_VIBRATE_STRENGTH,
                minimum_duration_seconds: MIN_VIBRATE_DURATION,
                maximum_duration_seconds: MIN_VIBRATE_DURATION,
            },
            fixed: VibrateFixedSettings {
                strength: MIN_VIBRATE_STRENGTH,
                duration_seconds: MIN_VIBRATE_DURATION,
            },
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolvedVibrateAction {
    pub strength: u8,
    pub duration_secs: u32,
}
impl ResolvedVibrateAction {
    pub fn summary(self) -> String {
        format!("{}/20 for {} s", self.strength, self.duration_secs)
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionValidationError {
    InvalidDuration,
    InvalidInterval,
    InvalidStrength,
}
impl fmt::Display for ActionValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDuration => {
                write!(f, "duration is out of the supported range for this action")
            }
            Self::InvalidInterval => write!(f, "interval minimum must not exceed maximum"),
            Self::InvalidStrength => {
                write!(f, "vibration strength must be an integer from 0 to 20")
            }
        }
    }
}
fn format_seconds(seconds: f32) -> String {
    if (seconds - 1.0).abs() < f32::EPSILON {
        "1 second".to_owned()
    } else {
        format!("{:.0} seconds", seconds)
    }
}
/// Renders a strength range as a single value when both ends match, since a
/// "3-3" range reads as a typo rather than a deliberate fixed value.
fn format_strength_range(minimum: f32, maximum: f32) -> String {
    if (minimum - maximum).abs() < f32::EPSILON {
        format!("{:.0}", minimum)
    } else {
        format!("{:.0} to {:.0}", minimum, maximum)
    }
}
/// Same as [`format_strength_range`], for the duration range.
fn format_duration_range(minimum: f32, maximum: f32) -> String {
    if (minimum - maximum).abs() < f32::EPSILON {
        format_seconds(minimum)
    } else {
        format!("{:.0}-{:.0} seconds", minimum, maximum)
    }
}
impl VibrateActionSettings {
    /// Plain-language description of the configured effect, shown to users in
    /// the trigger list who may not know what a compact "3-3/20 for 1-1s"
    /// shorthand means.
    pub fn summary(&self) -> String {
        match self.mode {
            VibrateMode::Fixed => format!(
                "Strength {:.0}, for {}",
                self.fixed.strength,
                format_seconds(self.fixed.duration_seconds)
            ),
            VibrateMode::Interval => format!(
                "Random strength {}, for {}",
                format_strength_range(
                    self.interval.minimum_strength,
                    self.interval.maximum_strength
                ),
                format_duration_range(
                    self.interval.minimum_duration_seconds,
                    self.interval.maximum_duration_seconds
                )
            ),
        }
    }
    pub fn resolve(&self) -> Option<ResolvedVibrateAction> {
        self.resolve_checked().ok()
    }
    pub fn resolve_checked(&self) -> Result<ResolvedVibrateAction, ActionValidationError> {
        let mut rng = rand::rng();
        self.resolve_with(&mut rng)
    }
    pub fn resolve_with<R: Rng + ?Sized>(
        &self,
        rng: &mut R,
    ) -> Result<ResolvedVibrateAction, ActionValidationError> {
        let strength = match self.mode {
            VibrateMode::Fixed => portable_strength(self.fixed.strength)
                .ok_or(ActionValidationError::InvalidStrength)?,
            VibrateMode::Interval => {
                let minimum = portable_strength(self.interval.minimum_strength)
                    .ok_or(ActionValidationError::InvalidStrength)?;
                let maximum = portable_strength(self.interval.maximum_strength)
                    .ok_or(ActionValidationError::InvalidStrength)?;
                if minimum > maximum {
                    return Err(ActionValidationError::InvalidInterval);
                }
                rng.random_range(minimum..=maximum)
            }
        };
        let duration_secs = match self.mode {
            VibrateMode::Fixed => portable_vibrate_duration(self.fixed.duration_seconds)
                .ok_or(ActionValidationError::InvalidDuration)?,
            VibrateMode::Interval => {
                let minimum = portable_vibrate_duration(self.interval.minimum_duration_seconds)
                    .ok_or(ActionValidationError::InvalidDuration)?;
                let maximum = portable_vibrate_duration(self.interval.maximum_duration_seconds)
                    .ok_or(ActionValidationError::InvalidDuration)?;
                if minimum > maximum {
                    return Err(ActionValidationError::InvalidInterval);
                }
                rng.random_range(minimum..=maximum)
            }
        };
        Ok(ResolvedVibrateAction {
            strength,
            duration_secs,
        })
    }
    pub fn copy_active_from(&mut self, source: &Self) {
        *self = source.clone();
    }
}
pub fn portable_strength(value: f32) -> Option<u8> {
    (value.is_finite()
        && value.fract() == 0.0
        && (MIN_VIBRATE_STRENGTH..=MAX_VIBRATE_STRENGTH).contains(&value))
    .then_some(value as u8)
}
pub fn portable_vibrate_duration(value: f32) -> Option<u32> {
    if !value.is_finite()
        || value.fract() != 0.0
        || !(MIN_VIBRATE_DURATION..=MAX_VIBRATE_DURATION).contains(&value)
    {
        return None;
    }
    Some(value as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    #[test]
    fn invalid_action_settings_return_typed_validation() {
        let mut settings = VibrateActionSettings::default();
        settings.mode = VibrateMode::Fixed;
        settings.fixed.strength = 21.0;
        assert_eq!(
            settings.resolve_checked(),
            Err(ActionValidationError::InvalidStrength)
        );
    }

    #[test]
    fn resolution_is_a_snapshot() {
        let mut settings = VibrateActionSettings::default();
        settings.mode = VibrateMode::Fixed;
        settings.fixed.strength = 12.0;
        settings.fixed.duration_seconds = 4.0;
        let mut rng = StdRng::seed_from_u64(4);
        let resolved = settings.resolve_with(&mut rng).unwrap();
        settings.fixed.strength = 20.0;
        assert_eq!(settings.fixed.strength, 20.0);
        assert_eq!(
            resolved,
            ResolvedVibrateAction {
                strength: 12,
                duration_secs: 4
            }
        );
    }

    #[test]
    fn interval_mode_rejects_inverted_bounds() {
        let mut settings = VibrateActionSettings::default();
        settings.mode = VibrateMode::Interval;
        settings.interval.minimum_strength = 10.0;
        settings.interval.maximum_strength = 5.0;
        assert_eq!(
            settings.resolve_checked(),
            Err(ActionValidationError::InvalidInterval)
        );
    }
}
