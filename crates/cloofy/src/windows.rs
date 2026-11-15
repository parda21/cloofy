//! `cloofy windows`: the Start Menu shortcut whose AppUserModelID Windows shows cloofy's toasts
//! under, and without which it shows none.

use std::{env, fs, io, path::Path, process};

use anyhow::{Context, Result};
use clap::Subcommand;
use cloofy_os::{AUMID, shortcut_path};
use windows::{
    Win32::{
        Storage::EnhancedStorage::PKEY_AppUserModel_ID,
        System::{
            Com::{
                CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemAlloc,
                IPersistFile, STGM_READ,
                StructuredStorage::{PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0, PropVariantClear},
            },
            Variant::VT_LPWSTR,
        },
        UI::Shell::{IShellLinkW, PropertiesSystem::IPropertyStore, ShellLink},
    },
    core::{HSTRING, Interface, PWSTR},
};

use crate::style;

/// Windows stores a shortcut's target path in at most `MAX_PATH` characters.
const MAX_PATH: usize = 260;

/// COM asks every thread which apartment it uses. `run_main` did it for the thread a command runs
/// on, and a test thread needs its own; a second call only says the apartment is already chosen.
fn init_com() {
    // SAFETY: no other thread's COM state is touched, and the code only reports a failure.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

#[derive(Subcommand)]
pub enum Action {
    /// Put cloofy in the Start Menu, where Windows finds it for notifications
    Install,
    /// Show whether that shortcut is there, and where it points
    Status,
    /// Take that shortcut out again
    Remove,
}

pub fn run(action: Action) -> Result<()> {
    let path = shortcut_path().context("APPDATA is not set, so there is no Start Menu to put it in")?;
    match action {
        Action::Install => install(&path),
        Action::Status => status(&path),
        Action::Remove => remove(&path),
    }
}

fn install(path: &Path) -> Result<()> {
    println!("\n  {}\n", style::bold("cloofy for Windows"));
    let exe = env::current_exe().context("Can't tell where cloofy is")?;
    write_shortcut(path, &exe)?;
    ok("Start Menu", &format!("{}, notifications show from it", path.display()));
    println!("\n  Done. {} takes it out again.\n", style::bold("cloofy windows remove"));
    Ok(())
}

fn status(path: &Path) -> Result<()> {
    println!("\n  {}\n", style::bold("cloofy for Windows"));
    if !path.is_file() {
        skipped("Start Menu", "not installed · `cloofy windows install` adds it");
        println!();
        process::exit(1);
    }
    let link = open_shortcut(path)?;
    let target = target_of(&link).unwrap_or_else(|| "unknown".into());
    let id = aumid_of(&link);
    match id.as_deref() {
        Some(id) if id == AUMID => ok("Start Menu", &format!("{target}, notifications show")),
        // A shortcut without the ID, or with another one, is a toast Windows never shows.
        Some(id) => skipped("Start Menu", &format!("{target}, but its AppUserModelID is {id}")),
        None => skipped("Start Menu", &format!("{target}, but it has no AppUserModelID")),
    }
    println!();
    if id.as_deref() != Some(AUMID) {
        println!("  {} brings it up to date.\n", style::bold("cloofy windows install"));
        process::exit(1);
    }
    Ok(())
}

fn remove(path: &Path) -> Result<()> {
    println!("\n  {}\n", style::bold("cloofy for Windows"));
    match fs::remove_file(path) {
        Ok(()) => ok("Start Menu", "removed"),
        Err(err) if err.kind() == io::ErrorKind::NotFound => skipped("Start Menu", "it wasn't there"),
        Err(err) => return Err(err).with_context(|| format!("Couldn't remove {}", path.display())),
    }
    println!();
    Ok(())
}

/// Writes the shortcut that points at `exe`, with cloofy's AppUserModelID on it.
fn write_shortcut(path: &Path, exe: &Path) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("Couldn't create {}", dir.display()))?;
    }
    init_com();
    let link: IShellLinkW = unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
        .context("Windows didn't give a shortcut to write")?;
    // SAFETY: every pointer below points at memory that outlives the call.
    unsafe {
        link.SetPath(&HSTRING::from(exe)).with_context(|| format!("The shortcut didn't take {}", exe.display()))?;
        link.SetDescription(&HSTRING::from("cloofy")).ok();
        let store: IPropertyStore = link.cast().context("The shortcut can't carry properties")?;
        let mut id = propvariant(AUMID);
        let set = store.SetValue(&PKEY_AppUserModel_ID, &id);
        let _ = PropVariantClear(&mut id);
        set.context("The shortcut didn't take cloofy's AppUserModelID")?;
        store.Commit().context("The shortcut's properties can't be saved")?;
        let file: IPersistFile = link.cast().context("The shortcut can't be written")?;
        file.Save(&HSTRING::from(path), true).with_context(|| format!("Couldn't write {}", path.display()))?;
    }
    Ok(())
}

/// The shortcut at `path`, read.
fn open_shortcut(path: &Path) -> Result<IShellLinkW> {
    init_com();
    let link: IShellLinkW = unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
        .context("Windows didn't give a shortcut to read")?;
    let file: IPersistFile = link.cast().context("Not a shortcut file")?;
    // SAFETY: the path outlives the call.
    unsafe { file.Load(&HSTRING::from(path), STGM_READ) }
        .with_context(|| format!("Couldn't read {}", path.display()))?;
    Ok(link)
}

/// Where the shortcut points.
fn target_of(link: &IShellLinkW) -> Option<String> {
    let mut target = [0u16; MAX_PATH];
    // SAFETY: the buffer outlives the call, and no find data is asked for.
    unsafe { link.GetPath(&mut target, std::ptr::null_mut(), 0) }.ok()?;
    let len = target.iter().position(|&c| c == 0).unwrap_or(target.len());
    Some(String::from_utf16_lossy(&target[..len]))
}

/// The shortcut's AppUserModelID.
fn aumid_of(link: &IShellLinkW) -> Option<String> {
    let store: IPropertyStore = link.cast().ok()?;
    let mut value = unsafe { store.GetValue(&PKEY_AppUserModel_ID) }.ok()?;
    // SAFETY: the string is read only under the type that makes it one, and freed below.
    let id = unsafe {
        let value = &*value.Anonymous.Anonymous;
        match value.vt == VT_LPWSTR {
            true => value.Anonymous.pwszVal.to_string().ok(),
            false => None,
        }
    };
    let _ = unsafe { PropVariantClear(&mut value) };
    id
}

/// `text` as the property store takes a string: a `VT_LPWSTR` the system's allocator holds, for
/// `PropVariantClear` to free.
fn propvariant(text: &str) -> PROPVARIANT {
    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let mut value = PROPVARIANT::default();
    // SAFETY: the memory is allocated by the allocator `PropVariantClear` frees it with.
    unsafe {
        let memory = CoTaskMemAlloc(wide.len() * size_of::<u16>()) as *mut u16;
        if memory.is_null() {
            return value;
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), memory, wide.len());
        value.Anonymous = PROPVARIANT_0 {
            Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_LPWSTR,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 { pwszVal: PWSTR(memory) },
            }),
        };
    }
    value
}

fn ok(name: &str, detail: &str) {
    println!("  {}  {name:<16}{}", style::green("✓"), style::dim(detail));
}

fn skipped(name: &str, detail: &str) {
    println!("  {}", style::dim(&format!("–  {name:<16}{detail}")));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_aumid_is_the_one_the_notifications_use() {
        assert_eq!(AUMID, "dev.cloofy.cloofy");
    }

    #[test]
    fn a_shortcut_can_be_written_read_and_removed() {
        let dir = env::temp_dir().join(format!("cloofy-windows-shortcut-{}", process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("cloofy.lnk");
        let exe = env::current_exe().unwrap();

        write_shortcut(&path, &exe).unwrap();
        let link = open_shortcut(&path).unwrap();
        assert_eq!(target_of(&link).as_deref(), Some(exe.to_string_lossy().as_ref()));
        assert_eq!(aumid_of(&link).as_deref(), Some(AUMID));

        fs::remove_dir_all(dir).unwrap();
    }
}
