use objc2::available;
use objc2_app_kit::NSWorkspace;
use objc2_foundation::NSProcessInfo;

use crate::settings::Settings;

pub fn current() -> Settings {
    Settings {
        reduce_motion: NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion(),
        // Macs have Low Power Mode since macOS 12.
        low_power: available!(macos = 12.0) && NSProcessInfo::processInfo().isLowPowerModeEnabled(),
    }
}
