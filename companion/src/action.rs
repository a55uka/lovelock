use rand::Rng;
use std::fmt;

pub const MIN_SHOCK_INTENSITY: f32 = 1.0;
pub const MAX_SHOCK_INTENSITY: f32 = 100.0;
pub const MIN_SHOCK_DURATION: f32 = 0.3;
pub const MAX_SHOCK_DURATION: f32 = 3.0;

pub const MIN_VIBRATE_STRENGTH: f32 = 0.0;
pub const MAX_VIBRATE_STRENGTH: f32 = 20.0;
pub const MIN_VIBRATE_DURATION: f32 = 1.0;
pub const MAX_VIBRATE_DURATION: f32 = 30.0;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ShockMode {
    #[default]
    Interval,
    Fixed,
}
impl ShockMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Interval => "Interval",
            Self::Fixed => "Fixed",
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ShockIntervalSettings {
    pub minimum_intensity: f32,
    pub maximum_intensity: f32,
    pub minimum_duration_seconds: f32,
    pub maximum_duration_seconds: f32,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ShockFixedSettings {
    pub intensity: f32,
    pub duration_seconds: f32,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ShockActionSettings {
    pub mode: ShockMode,
    pub interval: ShockIntervalSettings,
    pub fixed: ShockFixedSettings,
}
impl Default for ShockActionSettings {
    fn default() -> Self {
        Self {
            mode: ShockMode::default(),
            interval: ShockIntervalSettings {
                minimum_intensity: MIN_SHOCK_INTENSITY,
                maximum_intensity: MIN_SHOCK_INTENSITY,
                minimum_duration_seconds: MIN_SHOCK_DURATION,
                maximum_duration_seconds: MIN_SHOCK_DURATION,
            },
            fixed: ShockFixedSettings {
                intensity: MIN_SHOCK_INTENSITY,
                duration_seconds: MIN_SHOCK_DURATION,
            },
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolvedShockAction {
    pub intensity: u8,
    pub duration_ms: u64,
}
impl ResolvedShockAction {
    pub fn summary(self) -> String {
        format!(
            "{}% for {:.1} s",
            self.intensity,
            self.duration_ms as f32 / 1_000.0
        )
    }
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum VibrateMode {
    #[default]
    Interval,
    Fixed,
}
impl VibrateMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Interval => "Interval",
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionKind {
    Shock,
    Vibrate,
}
impl ActionKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Shock => "shock",
            Self::Vibrate => "vibrate",
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionValidationError {
    Unsupported { action: ActionKind },
    InvalidIntensity,
    InvalidDuration,
    InvalidInterval,
    InvalidStrength,
}
impl fmt::Display for ActionValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported { action } => write!(f, "unsupported {} action", action.label()),
            Self::InvalidIntensity => write!(f, "shock intensity must be an integer from 1 to 100"),
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolvedAction {
    Shock(ResolvedShockAction),
    Vibrate(ResolvedVibrateAction),
}
impl ResolvedAction {
    pub fn kind(self) -> ActionKind {
        match self {
            Self::Shock(_) => ActionKind::Shock,
            Self::Vibrate(_) => ActionKind::Vibrate,
        }
    }
    pub fn summary(self) -> String {
        match self {
            Self::Shock(shock) => format!("Shock {}", shock.summary()),
            Self::Vibrate(vibrate) => format!("Vibrate {}", vibrate.summary()),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ActionSettings {
    pub shock: ShockActionSettings,
    pub vibrate: VibrateActionSettings,
}
impl Default for ActionSettings {
    fn default() -> Self {
        Self {
            shock: ShockActionSettings::default(),
            vibrate: VibrateActionSettings::default(),
        }
    }
}
impl ActionSettings {
    pub fn summary(&self, kind: ActionKind) -> String {
        match kind {
            ActionKind::Shock => self.shock.summary(),
            ActionKind::Vibrate => self.vibrate.summary(),
        }
    }
    pub fn resolve(&self, kind: ActionKind) -> Result<ResolvedAction, ActionValidationError> {
        match kind {
            ActionKind::Shock => self.shock.resolve_checked().map(ResolvedAction::Shock),
            ActionKind::Vibrate => self.vibrate.resolve_checked().map(ResolvedAction::Vibrate),
        }
    }
    pub fn copy_active_from(&mut self, source: &Self, kind: ActionKind) {
        match kind {
            ActionKind::Shock => self.shock = source.shock.clone(),
            ActionKind::Vibrate => self.vibrate = source.vibrate.clone(),
        }
    }
    pub fn active_editor_kind(&self, kind: ActionKind) -> ActionKind {
        kind
    }
}
impl ShockActionSettings {
    pub fn resolve(&self) -> Option<ResolvedShockAction> {
        self.resolve_checked().ok()
    }
    pub fn resolve_checked(&self) -> Result<ResolvedShockAction, ActionValidationError> {
        let mut rng = rand::rng();
        self.resolve_with(&mut rng)
    }
    pub fn resolve_with<R: Rng + ?Sized>(
        &self,
        rng: &mut R,
    ) -> Result<ResolvedShockAction, ActionValidationError> {
        let intensity = match self.mode {
            ShockMode::Fixed => portable_intensity(self.fixed.intensity)
                .ok_or(ActionValidationError::InvalidIntensity)?,
            ShockMode::Interval => {
                let minimum = portable_intensity(self.interval.minimum_intensity)
                    .ok_or(ActionValidationError::InvalidIntensity)?;
                let maximum = portable_intensity(self.interval.maximum_intensity)
                    .ok_or(ActionValidationError::InvalidIntensity)?;
                if minimum > maximum {
                    return Err(ActionValidationError::InvalidInterval);
                }
                rng.random_range(minimum..=maximum)
            }
        };
        let duration_ms = match self.mode {
            ShockMode::Fixed => portable_duration(self.fixed.duration_seconds)
                .ok_or(ActionValidationError::InvalidDuration)?,
            ShockMode::Interval => {
                let minimum = portable_duration(self.interval.minimum_duration_seconds)
                    .ok_or(ActionValidationError::InvalidDuration)?;
                let maximum = portable_duration(self.interval.maximum_duration_seconds)
                    .ok_or(ActionValidationError::InvalidDuration)?;
                if minimum > maximum {
                    return Err(ActionValidationError::InvalidInterval);
                }
                rng.random_range(minimum..=maximum)
            }
        };
        Ok(ResolvedShockAction {
            intensity,
            duration_ms,
        })
    }
    pub fn summary(&self) -> String {
        match self.mode {
            ShockMode::Fixed => format!(
                "{:.0}% for {:.1} s",
                self.fixed.intensity, self.fixed.duration_seconds
            ),
            ShockMode::Interval => format!(
                "{:.0}–{:.0}% for {:.1}–{:.1} s",
                self.interval.minimum_intensity,
                self.interval.maximum_intensity,
                self.interval.minimum_duration_seconds,
                self.interval.maximum_duration_seconds
            ),
        }
    }
}
pub fn portable_intensity(value: f32) -> Option<u8> {
    (value.is_finite()
        && value.fract() == 0.0
        && (MIN_SHOCK_INTENSITY..=MAX_SHOCK_INTENSITY).contains(&value))
    .then_some(value as u8)
}
pub fn portable_duration(value: f32) -> Option<u64> {
    if !value.is_finite() || !(MIN_SHOCK_DURATION..=MAX_SHOCK_DURATION).contains(&value) {
        return None;
    }
    Some((value * 1_000.0).round() as u64)
}
impl VibrateActionSettings {
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
            VibrateMode::Fixed => {
                portable_strength(self.fixed.strength).ok_or(ActionValidationError::InvalidStrength)?
            }
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
    pub fn summary(&self) -> String {
        match self.mode {
            VibrateMode::Fixed => format!(
                "{:.0}/20 for {:.0} s",
                self.fixed.strength, self.fixed.duration_seconds
            ),
            VibrateMode::Interval => format!(
                "{:.0}–{:.0}/20 for {:.0}–{:.0} s",
                self.interval.minimum_strength,
                self.interval.maximum_strength,
                self.interval.minimum_duration_seconds,
                self.interval.maximum_duration_seconds
            ),
        }
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
    fn both_current_providers_share_the_shock_action_family() {
        assert_eq!(
            crate::provider::ProviderKind::PiShock.action_kind(),
            ActionKind::Shock
        );
        assert_eq!(
            crate::provider::ProviderKind::OpenShock.action_kind(),
            ActionKind::Shock
        );
    }

    #[test]
    fn invalid_action_settings_return_typed_validation() {
        let mut settings = ActionSettings::default();
        settings.shock.mode = ShockMode::Fixed;
        settings.shock.fixed.intensity = 0.0;
        assert_eq!(
            settings.resolve(ActionKind::Shock),
            Err(ActionValidationError::InvalidIntensity)
        );
    }

    #[test]
    fn resolution_is_a_snapshot() {
        let mut settings = ShockActionSettings::default();
        settings.mode = ShockMode::Fixed;
        settings.fixed.intensity = 41.0;
        settings.fixed.duration_seconds = 1.2;
        let mut rng = StdRng::seed_from_u64(4);
        let resolved = settings.resolve_with(&mut rng).unwrap();
        settings.fixed.intensity = 99.0;
        assert_eq!(settings.fixed.intensity, 99.0);
        assert_eq!(
            resolved,
            ResolvedShockAction {
                intensity: 41,
                duration_ms: 1_200
            }
        );
    }
}
