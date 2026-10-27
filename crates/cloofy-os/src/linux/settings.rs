//! Power saving from power-profiles-daemon, less motion from the desktop's settings portal.
//! Both are watched in the background, so reading them costs next to nothing.

use std::{
    fs,
    sync::{
        Once,
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use futures::StreamExt;
use zbus::{Proxy, proxy, zvariant::Value};

use super::bus;
use crate::settings::Settings;

/// How long the first read waits for the background, so cloofy starts with the right settings.
const FIRST_READ: Duration = Duration::from_millis(250);
/// power-profiles-daemon by its current name and the one it had before 0.20, with the object
/// and interface of each.
const PROFILES: [(&str, &str); 2] = [
    ("org.freedesktop.UPower.PowerProfiles", "/org/freedesktop/UPower/PowerProfiles"),
    ("net.hadess.PowerProfiles", "/net/hadess/PowerProfiles"),
];
/// The kernel's profile, for systems without power-profiles-daemon.
const PLATFORM_PROFILE: &str = "/sys/firmware/acpi/platform_profile";

/// The active power profile, as far as power-profiles-daemon reported it.
static POWER_SAVER: AtomicU8 = AtomicU8::new(UNKNOWN);
const UNKNOWN: u8 = 0;
const NO: u8 = 1;
const YES: u8 = 2;
static REDUCE_MOTION: AtomicBool = AtomicBool::new(false);

pub fn current() -> Settings {
    static WATCH: Once = Once::new();
    WATCH.call_once(|| {
        let (ready, first_read) = mpsc::channel();
        bus::spawn(watch_power_profile(ready.clone()));
        bus::spawn(watch_motion(ready));
        // Each watch drops its sender once it read its setting, or gave up.
        let deadline = Instant::now() + FIRST_READ;
        while first_read.recv_timeout(deadline.saturating_duration_since(Instant::now())).is_ok() {}
    });
    let low_power = match POWER_SAVER.load(Ordering::Relaxed) {
        UNKNOWN => fs::read_to_string(PLATFORM_PROFILE).is_ok_and(|profile| profile.trim() == "low-power"),
        saver => saver == YES,
    };
    Settings { reduce_motion: REDUCE_MOTION.load(Ordering::Relaxed), low_power }
}

/// Follows power-profiles-daemon's active profile, under either name.
async fn watch_power_profile(ready: mpsc::Sender<()>) {
    let Some(system) = bus::system().await else { return };
    for (name, path) in PROFILES {
        // Both name their interface like the service.
        let Ok(proxy) = Proxy::new(&system, name, path, name).await else { continue };
        let Ok(profile) = proxy.get_property::<String>("ActiveProfile").await else { continue };
        set_power_saver(&profile);
        drop(ready);
        // Starts with the current profile again, then each change.
        let mut changes = proxy.receive_property_changed::<String>("ActiveProfile").await;
        while let Some(change) = changes.next().await {
            if let Ok(profile) = change.get().await {
                set_power_saver(&profile);
            }
        }
        return;
    }
    tracing::debug!("no power-profiles-daemon; following {PLATFORM_PROFILE}");
}

fn set_power_saver(profile: &str) {
    POWER_SAVER.store(if profile == "power-saver" { YES } else { NO }, Ordering::Relaxed);
}

#[proxy(
    interface = "org.freedesktop.portal.Settings",
    default_service = "org.freedesktop.portal.Desktop",
    default_path = "/org/freedesktop/portal/desktop"
)]
trait Portal {
    fn read_one(&self, namespace: &str, key: &str) -> zbus::Result<zbus::zvariant::OwnedValue>;

    /// What `ReadOne` replaced in version 2 of the interface, with the value in another variant.
    fn read(&self, namespace: &str, key: &str) -> zbus::Result<zbus::zvariant::OwnedValue>;

    #[zbus(signal)]
    fn setting_changed(&self, namespace: &str, key: &str, value: Value<'_>) -> zbus::Result<()>;
}

/// The settings that ask for less motion: the standard one, which portals have since 1.21, and
/// GNOME's own for older ones. Either is enough.
const REDUCED_MOTION: (&str, &str) = ("org.freedesktop.appearance", "reduced-motion");
const ANIMATIONS: (&str, &str) = ("org.gnome.desktop.interface", "enable-animations");

/// Follows the desktop's wish for less motion through the settings portal.
async fn watch_motion(ready: mpsc::Sender<()>) {
    let Some(session) = bus::session().await else { return };
    let Ok(portal) = PortalProxy::new(&session).await else { return };
    // Listening first, so no change gets lost between reading and listening.
    let Ok(mut changes) = portal.receive_setting_changed().await else { return };
    let mut motion = Motion::default();
    for (namespace, key) in [REDUCED_MOTION, ANIMATIONS] {
        let value = match portal.read_one(namespace, key).await {
            Ok(value) => Ok(value),
            Err(_) => portal.read(namespace, key).await,
        };
        if let Ok(value) = value {
            motion.set(namespace, key, &value);
        }
    }
    REDUCE_MOTION.store(motion.reduce(), Ordering::Relaxed);
    drop(ready);
    while let Some(change) = changes.next().await {
        if let Ok(args) = change.args()
            && motion.set(args.namespace(), args.key(), args.value())
        {
            REDUCE_MOTION.store(motion.reduce(), Ordering::Relaxed);
        }
    }
}

/// What the desktop says about motion.
#[derive(Debug, Default)]
struct Motion {
    reduced: bool,
    animations_off: bool,
}

impl Motion {
    /// Takes in a setting; returns whether it was one about motion.
    fn set(&mut self, namespace: &str, key: &str, value: &Value<'_>) -> bool {
        match (namespace, key, value) {
            // `Read` wraps the value once more.
            (_, _, Value::Value(inner)) => self.set(namespace, key, inner),
            // 1 asks for less motion; unknown values count as no preference.
            (_, _, Value::U32(preference)) if (namespace, key) == REDUCED_MOTION => {
                self.reduced = *preference == 1;
                true
            }
            (_, _, Value::Bool(enabled)) if (namespace, key) == ANIMATIONS => {
                self.animations_off = !enabled;
                true
            }
            _ => false,
        }
    }

    fn reduce(&self) -> bool {
        self.reduced || self.animations_off
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_either_motion_setting() {
        let mut motion = Motion::default();
        assert!(motion.set("org.freedesktop.appearance", "reduced-motion", &Value::U32(1)));
        assert!(motion.reduce());
        motion.set("org.freedesktop.appearance", "reduced-motion", &Value::U32(0));
        assert!(!motion.reduce());
        motion.set("org.gnome.desktop.interface", "enable-animations", &Value::Bool(false));
        assert!(motion.reduce());
        motion.set("org.gnome.desktop.interface", "enable-animations", &Value::Bool(true));
        assert!(!motion.reduce());
    }

    #[test]
    fn reads_values_from_older_portals() {
        let mut motion = Motion::default();
        let wrapped = Value::Value(Box::new(Value::U32(1)));
        assert!(motion.set("org.freedesktop.appearance", "reduced-motion", &wrapped));
        assert!(motion.reduce());
    }

    #[test]
    fn ignores_other_settings() {
        let mut motion = Motion::default();
        assert!(!motion.set("org.freedesktop.appearance", "color-scheme", &Value::U32(1)));
        assert!(!motion.set("org.gnome.desktop.interface", "enable-animations", &Value::U32(0)));
        // Values the standard doesn't know count as no preference.
        motion.set("org.freedesktop.appearance", "reduced-motion", &Value::U32(7));
        assert!(!motion.reduce());
    }
}
