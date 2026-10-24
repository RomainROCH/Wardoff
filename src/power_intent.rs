//! Shared intent for manual idle-power protection; no Windows effects.

use serde::{Deserialize, Serialize};

/// Selects the owned Power Request set while Wardoff is in Block mode.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerIntent {
    /// Prevent idle system sleep while allowing the display's normal timeout.
    SystemOnly,
    /// Preserve the default idle-system and display-timeout prevention.
    #[default]
    SystemAndDisplay,
}

impl PowerIntent {
    /// Returns system/display request selections in backend acquisition order.
    pub(crate) fn request_mask(self) -> [bool; 2] {
        [true, self == Self::SystemAndDisplay]
    }
}

#[cfg(test)]
mod tests {
    use super::PowerIntent;

    #[test]
    fn wire_intent_names_are_stable_and_reject_display_only_or_unknown_values() {
        for (intent, json) in [
            (PowerIntent::SystemOnly, "\"system_only\""),
            (PowerIntent::SystemAndDisplay, "\"system_and_display\""),
        ] {
            assert_eq!(serde_json::to_string(&intent).unwrap(), json);
            assert_eq!(serde_json::from_str::<PowerIntent>(json).unwrap(), intent);
        }
        for invalid in ["\"display_only\"", "\"unknown\"", "null", "true"] {
            assert!(serde_json::from_str::<PowerIntent>(invalid).is_err());
        }
    }
}
