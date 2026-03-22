use tray_icon::{BadIcon, Icon};

/// Represents the visual states supported by the tray icon assets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrayIconState {
    /// Indicates that blocking is enabled.
    Blocked,
    /// Indicates that allowing shutdown and sleep is enabled.
    Allowed,
}

const ICON_SIZE: u32 = 32;
const OUTER_RADIUS: f32 = 12.5;
const INNER_RADIUS: f32 = 9.5;
const BORDER_COLOR: [u8; 4] = [32, 32, 32, 255];
const BLOCKED_FILL: [u8; 4] = [217, 72, 72, 255];
const ALLOWED_FILL: [u8; 4] = [52, 168, 83, 255];

/// Creates the red tray icon used while Wardoff is blocking.
pub fn blocked_icon() -> Result<Icon, BadIcon> {
    icon_for_state(TrayIconState::Blocked)
}

/// Creates the green tray icon used while Wardoff is allowing shutdown and sleep.
pub fn allowed_icon() -> Result<Icon, BadIcon> {
    icon_for_state(TrayIconState::Allowed)
}

/// Creates the tray icon bitmap for the requested Wardoff mode.
pub fn icon_for_state(state: TrayIconState) -> Result<Icon, BadIcon> {
    let fill = match state {
        TrayIconState::Blocked => BLOCKED_FILL,
        TrayIconState::Allowed => ALLOWED_FILL,
    };

    let mut rgba = vec![0u8; (ICON_SIZE * ICON_SIZE * 4) as usize];
    let center = (ICON_SIZE as f32 - 1.0) / 2.0;

    for y in 0..ICON_SIZE {
        for x in 0..ICON_SIZE {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let distance = (dx * dx + dy * dy).sqrt();
            let pixel = if distance <= INNER_RADIUS {
                fill
            } else if distance <= OUTER_RADIUS {
                BORDER_COLOR
            } else {
                [0, 0, 0, 0]
            };

            let offset = ((y * ICON_SIZE + x) * 4) as usize;
            rgba[offset..offset + 4].copy_from_slice(&pixel);
        }
    }

    Icon::from_rgba(rgba, ICON_SIZE, ICON_SIZE)
}
