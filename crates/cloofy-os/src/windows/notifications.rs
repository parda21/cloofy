//! Song notifications as Windows toasts, with the cover.
//!
//! Windows only shows toasts of an app it knows from a Start Menu shortcut carrying its
//! AppUserModelID; without one it accepts the notifier and shows nothing. So `new` says no
//! without the shortcut that `cloofy windows install` makes, rather than pretending it worked.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process,
    sync::{Arc, Mutex, PoisonError},
};

use windows::{
    Data::Xml::Dom::XmlDocument,
    UI::Notifications::{ToastNotification, ToastNotificationManager, ToastNotifier},
    core::{HSTRING, Result as WinResult},
};

use super::{AUMID, shortcut_path};

/// The toast the next one replaces: Windows keeps one per tag and group.
const TAG: &str = "now-playing";
const GROUP: &str = "cloofy";

pub struct Notifications {
    notifier: ToastNotifier,
    /// The cover of the toast on screen, which its replacement takes away.
    shown: Mutex<Shown>,
}

#[derive(Default)]
struct Shown {
    image: Option<PathBuf>,
    written: u64,
}

/// Whether a click on a notification brings cloofy's window to the front. It would take a COM
/// activator and a new instance, which a terminal interface has nowhere to bring forward.
pub fn click_raises() -> bool {
    false
}

impl Notifications {
    /// `None` without the Start Menu shortcut, which Windows needs to show the toast at all.
    pub fn new() -> Option<Self> {
        let shortcut = shortcut_path()?;
        if !shortcut.is_file() {
            tracing::debug!("No notifications without {}: `cloofy windows install` makes it", shortcut.display());
            return None;
        }
        let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))
            .inspect_err(|err| tracing::debug!("Windows refused the toast notifier: {err}"))
            .ok()?;
        Some(Self { notifier, shown: Mutex::default() })
    }

    /// Shows `title` and `body`, with `image` (JPEG, PNG or WebP) where there is one, in place of
    /// the notification shown before.
    pub fn show(&self, title: &str, body: &str, image: Option<Arc<[u8]>>) {
        let image = image.and_then(|image| {
            let mut shown = self.shown.lock().unwrap_or_else(PoisonError::into_inner);
            shown.write(&image)
        });
        match toast(title, body, image.as_deref()) {
            Ok(toast) => {
                if let Err(err) = self.notifier.Show(&toast) {
                    tracing::warn!("notification: {err}");
                }
            }
            Err(err) => tracing::warn!("notification: {err}"),
        }
    }
}

impl Drop for Notifications {
    fn drop(&mut self) {
        let image = self.shown.lock().unwrap_or_else(PoisonError::into_inner).image.take();
        if let Some(path) = image {
            let _ = fs::remove_file(path);
        }
    }
}

impl Shown {
    /// Writes `image` to a file of its own, a toast taking images by path, and removes the one
    /// before: the notification it belonged to is being replaced.
    fn write(&mut self, image: &[u8]) -> Option<PathBuf> {
        if let Some(old) = self.image.take() {
            let _ = fs::remove_file(old);
        }
        self.written += 1;
        // The shell reads the file by itself, so it can't live in cloofy's process; Windows
        // clears the temporary folder now and then, which is all this needs.
        let path = std::env::temp_dir().join(format!("cloofy-{}-notification-{}.img", process::id(), self.written));
        // A new file only, as Windows may still read the one before.
        let written = OpenOptions::new().write(true).create_new(true).open(&path).and_then(|mut f| f.write_all(image));
        if let Err(err) = written {
            tracing::warn!("cover for the notification: {}: {err}", path.display());
            let _ = fs::remove_file(&path);
            return None;
        }
        self.image = Some(path.clone());
        Some(path)
    }
}

/// The toast, tagged so the next one replaces it.
fn toast(title: &str, body: &str, image: Option<&Path>) -> WinResult<ToastNotification> {
    let xml = XmlDocument::new()?;
    xml.LoadXml(&HSTRING::from(toast_xml(title, body, image)))?;
    let toast = ToastNotification::CreateToastNotification(&xml)?;
    toast.SetTag(&HSTRING::from(TAG))?;
    toast.SetGroup(&HSTRING::from(GROUP))?;
    Ok(toast)
}

/// The toast's content: the title, the artist and album, and the cover where there is one.
fn toast_xml(title: &str, body: &str, image: Option<&Path>) -> String {
    let body = match body.is_empty() {
        true => String::new(),
        false => format!("<text>{}</text>", escape(body)),
    };
    let image = match image {
        Some(path) => {
            let path = path.display().to_string().replace('\\', "/");
            format!(r#"<image placement="appLogoOverride" src="file:///{path}"/>"#)
        }
        None => String::new(),
    };
    format!(
        "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text>{body}{image}</binding></visual></toast>",
        escape(title)
    )
}

/// XML reads `&` and `<` as markup, so text that may hold them is escaped first.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_what_looks_like_markup() {
        assert_eq!(escape("Simon & Garfunkel <live>"), "Simon &amp; Garfunkel &lt;live&gt;");
        assert_eq!(escape(r#"a "quoted" 'name'"#), "a &quot;quoted&quot; &apos;name&apos;");
    }

    #[test]
    fn puts_the_cover_and_the_text_in_the_toast() {
        let cover = Path::new(r"C:\Users\sam\cloofy\cover.jpg");
        let xml = toast_xml("Strobe", "deadmau5 · For Lack of a Better Name", Some(cover));
        assert!(xml.starts_with("<toast><visual><binding template=\"ToastGeneric\">"));
        assert!(xml.contains("<text>Strobe</text>"));
        assert!(xml.contains("<text>deadmau5 · For Lack of a Better Name</text>"));
        assert!(xml.contains(r#"<image placement="appLogoOverride" src="file:///C:/Users/sam/cloofy/cover.jpg"/>"#));
        let xml = toast_xml("Strobe", "", None);
        assert_eq!(xml.matches("<text>").count(), 1, "no empty line without an album:\n{xml}");
        assert!(!xml.contains("<image"));
    }

    #[test]
    fn keeps_one_cover_file() {
        let mut shown = Shown::default();
        let first = shown.write(b"\xff\xd8\xffjpeg").expect("written");
        let second = shown.write(b"\x89PNG\r\n\x1a\npng").expect("written");
        assert!(!first.exists(), "the replaced notification's cover is gone");
        assert_eq!(fs::read(&second).unwrap(), b"\x89PNG\r\n\x1a\npng");
        fs::remove_file(second).unwrap();
    }

    /// Shows a real toast: run it by hand with
    /// `cargo test -p cloofy-os --lib -- --ignored shows_a_toast --nocapture`.
    #[test]
    #[ignore = "shows a toast on the desktop"]
    fn shows_a_toast() {
        use windows::UI::Notifications::ToastNotificationManager;

        let notifications = Notifications::new().expect("the Start Menu shortcut is installed");
        println!("setting: {:?}", notifications.notifier.Setting().map(|setting| setting.0));
        let toast = toast("cloofy toast probe", "song notifications look like this", None).unwrap();
        println!("show: {:?}", notifications.notifier.Show(&toast));
        let history = ToastNotificationManager::History().unwrap();
        let items = history.GetHistoryWithId(&HSTRING::from(AUMID)).unwrap();
        println!("history: {}", items.Size().unwrap());
        for item in items {
            println!("{}", item.Content().unwrap().GetXml().unwrap());
        }
    }
}
