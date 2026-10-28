//! `cloofy omarchy`: cloofy as part of Omarchy. A widget in the bar, colors that follow the
//! theme, entries in the Omarchy menu, a launcher, and on request Super+Shift+M.
//!
//! Everything it adds is either a file of cloofy's own or a block between marker comments in a
//! file of the user's, so `remove` takes out exactly that and leaves the rest as it was.

use std::{
    collections::HashMap,
    env, fs,
    io::{self, BufRead, IsTerminal, Write},
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use clap::Subcommand;
use cloofy_tui::omarchy as theme;

use crate::style;

const PLUGIN_ID: &str = "cloofy.nowplaying";
const PLUGIN: [(&str, &str); 4] = [
    ("manifest.json", include_str!("../../../integrations/omarchy/plugin/manifest.json")),
    ("Widget.qml", include_str!("../../../integrations/omarchy/plugin/Widget.qml")),
    ("Equalizer.qml", include_str!("../../../integrations/omarchy/plugin/Equalizer.qml")),
    ("Marquee.qml", include_str!("../../../integrations/omarchy/plugin/Marquee.qml")),
];
const ICON: &str = include_str!("../../../integrations/omarchy/cloofy.svg");
/// Next to the clock, which stays in the middle of the bar.
const PLACEMENT: &str = r#"{"section":"center","after":"omarchy.clock"}"#;
/// How long a freshly copied plugin may take until the shell knows it.
const SHELL_SCAN: Duration = Duration::from_secs(5);

const BEGIN: &str = "cloofy: begin";
const END: &str = "cloofy: end";
/// Says so in the block when cloofy created the whole file, which then goes with the block.
const CREATED: &str = "cloofy created this file";

/// Marks the desktop entry as the one `install` wrote.
const DESKTOP_MARK: &str = "X-cloofy-omarchy=true";
const DESKTOP_ENTRY: &str = "\
[Desktop Entry]
Version=1.0
Type=Application
Name=cloofy
GenericName=Music Player
Comment=Spotify and YouTube Music in your terminal
Exec=omarchy-launch-or-focus-tui cloofy
Icon=cloofy
Terminal=false
Categories=Audio;AudioVideo;Player;
Keywords=music;spotify;youtube;player;
StartupWMClass=org.omarchy.cloofy
X-cloofy-omarchy=true
";

const MENU_ENTRIES: &str = r#"  "music": {"icon":"󰝚","label":"Music","aliases":["cloofy"]},
  "music.open": {"icon":"󰆍","label":"cloofy","description":"Open cloofy, or bring it to the front","action":"omarchy-launch-or-focus-tui cloofy"},
  "music.play-pause": {"icon":"󰐎","label":"Play/pause","when":"busctl --user status org.mpris.MediaPlayer2.cloofy","action":"busctl --user call org.mpris.MediaPlayer2.cloofy /org/mpris/MediaPlayer2 org.mpris.MediaPlayer2.Player PlayPause"},
  "music.next": {"icon":"󰒭","label":"Next","when":"busctl --user status org.mpris.MediaPlayer2.cloofy","action":"busctl --user call org.mpris.MediaPlayer2.cloofy /org/mpris/MediaPlayer2 org.mpris.MediaPlayer2.Player Next"},
  "music.previous": {"icon":"󰒮","label":"Previous","when":"busctl --user status org.mpris.MediaPlayer2.cloofy","action":"busctl --user call org.mpris.MediaPlayer2.cloofy /org/mpris/MediaPlayer2 org.mpris.MediaPlayer2.Player Previous"},
"#;

const BINDING: &str = r#"hl.unbind("SUPER + SHIFT + M")
o.bind("SUPER + SHIFT + M", "Music", { tui = "cloofy", focus = true })
"#;

#[derive(Subcommand)]
pub enum Action {
    /// Add the bar widget, the theme's colors, the menu entries and the launcher
    Install {
        /// Also make Super+Shift+M open cloofy instead of Spotify
        #[arg(long)]
        bind_music_key: bool,
        /// Don't ask: everything, Super+Shift+M included
        #[arg(short, long)]
        yes: bool,
    },
    /// Take out everything `install` added
    Remove,
    /// Show what is set up
    Status,
}

pub fn run(action: Action) -> Result<()> {
    let Some(omarchy) = Omarchy::find() else {
        println!("\n  {} Omarchy not found, so there is nothing to set up.\n", style::dim("–"));
        return Ok(());
    };
    match action {
        Action::Install { bind_music_key, yes } => {
            // Bound before, it stays bound without asking again.
            let bound = fs::read_to_string(omarchy.bindings()).is_ok_and(|text| has_block(&text, "--"));
            omarchy.install(bind_music_key || yes || bound || ask_music_key())
        }
        Action::Remove => omarchy.remove(),
        Action::Status => {
            omarchy.status();
            Ok(())
        }
    }
}

/// What a step did, or why it had nothing to do.
enum Step {
    Done(String),
    Nothing(String),
}

/// Where Omarchy keeps what cloofy adds to it.
struct Omarchy {
    home: PathBuf,
    /// `~/.local/state/omarchy`, with the current theme.
    state: PathBuf,
    /// Omarchy itself, for the themes it ships.
    path: PathBuf,
}

impl Omarchy {
    fn find() -> Option<Self> {
        let state = theme::state_dir()?;
        let home = PathBuf::from(env::var_os("HOME")?);
        let path = env::var_os("OMARCHY_PATH").map_or_else(|| PathBuf::from("/usr/share/omarchy"), PathBuf::from);
        Some(Self { home, state, path })
    }

    fn config(&self) -> PathBuf {
        self.home.join(".config/omarchy")
    }

    fn plugin_dir(&self) -> PathBuf {
        self.config().join("plugins").join(PLUGIN_ID)
    }

    fn template(&self) -> PathBuf {
        self.config().join("themed/cloofy.toml.tpl")
    }

    fn rendered(&self) -> PathBuf {
        self.state.join("current/theme/cloofy.toml")
    }

    fn menu(&self) -> PathBuf {
        self.config().join("extensions/omarchy-menu.jsonc")
    }

    fn desktop_entry(&self) -> PathBuf {
        self.home.join(".local/share/applications/cloofy.desktop")
    }

    fn icon(&self) -> PathBuf {
        self.home.join(".local/share/icons/hicolor/scalable/apps/cloofy.svg")
    }

    fn bindings(&self) -> PathBuf {
        self.home.join(".config/hypr/bindings.lua")
    }

    fn install(&self, bind_music_key: bool) -> Result<()> {
        println!("\n  {}\n", style::bold("cloofy for Omarchy"));
        report("Bar widget", self.install_widget());
        report("Theme", self.install_theme());
        report("Menu", self.install_menu());
        report("Launcher", self.install_launcher());
        if bind_music_key {
            report("Super+Shift+M", self.install_binding());
        } else {
            skipped("Super+Shift+M", "still opens Spotify · `cloofy omarchy install --bind-music-key`");
        }
        println!("\n  Done. {} takes it all out again.\n", style::bold("cloofy omarchy remove"));
        Ok(())
    }

    fn remove(&self) -> Result<()> {
        println!("\n  {}\n", style::bold("cloofy for Omarchy"));
        report("Bar widget", self.remove_widget());
        report("Theme", self.remove_theme());
        report("Menu", remove_block_from(&self.menu(), "//").map(|done| removed(done, "the Music entries")));
        report("Launcher", self.remove_launcher());
        report("Super+Shift+M", self.remove_binding());
        println!();
        Ok(())
    }

    fn status(&self) {
        println!("\n  {}\n", style::bold("cloofy for Omarchy"));
        let shell = shell_plugins();
        let in_bar = shell.as_deref().map(|plugins| plugin_enabled(plugins, PLUGIN_ID));
        match (self.plugin_dir().join("manifest.json").is_file(), in_bar) {
            (true, Some(true)) => ok("Bar widget", &format!("{PLUGIN_ID}, in the bar")),
            (true, Some(false)) => {
                skipped("Bar widget", "installed, not in the bar · `omarchy plugin enable cloofy.nowplaying`")
            }
            (true, None) => ok("Bar widget", &format!("{PLUGIN_ID}, installed")),
            (false, _) => skipped("Bar widget", "not installed"),
        }
        match (self.template().is_file(), self.rendered().is_file()) {
            (true, _) => ok("Theme", &format!("follows {}", self.theme_name())),
            (false, true) => ok("Theme", &format!("{} brings colors for cloofy", self.theme_name())),
            (false, false) => skipped("Theme", "cloofy derives its colors from the theme itself"),
        }
        let menu = fs::read_to_string(self.menu()).unwrap_or_default();
        if has_block(&menu, "//") {
            ok("Menu", "Music in the Omarchy menu");
        } else {
            skipped("Menu", "not installed");
        }
        if is_ours(&self.desktop_entry()) {
            ok("Launcher", "cloofy in the app launcher");
        } else {
            skipped("Launcher", "not installed");
        }
        let bindings = fs::read_to_string(self.bindings()).unwrap_or_default();
        if has_block(&bindings, "--") {
            ok("Super+Shift+M", "opens cloofy");
        } else {
            skipped("Super+Shift+M", "opens Spotify, as Omarchy has it");
        }
        println!();
    }

    fn install_widget(&self) -> Result<Step> {
        let dir = self.plugin_dir();
        fs::create_dir_all(&dir).with_context(|| format!("Couldn't create {}", dir.display()))?;
        for (name, content) in PLUGIN {
            write_if_changed(&dir.join(name), content)?;
        }
        if let Ok(output) = Command::new("omarchy-plugin-validate").arg(&dir).output()
            && !output.status.success()
        {
            anyhow::bail!("Omarchy refused the widget: {}", String::from_utf8_lossy(&output.stderr).trim());
        }
        if shell(&["rescanPlugins"]).is_none() {
            return Ok(Step::Done(format!(
                "copied; once the shell runs: {}",
                style::bold("omarchy plugin enable cloofy.nowplaying")
            )));
        }
        // The shell scans in the background; until it is done it doesn't know the widget yet.
        let deadline = Instant::now() + SHELL_SCAN;
        loop {
            match shell(&["putBarWidget", PLUGIN_ID, PLACEMENT]).as_deref() {
                Some("ok") => return Ok(Step::Done(format!("{PLUGIN_ID}, in the bar"))),
                Some(answer) if Instant::now() >= deadline => anyhow::bail!("the shell said: {answer}"),
                None => anyhow::bail!("the shell stopped answering"),
                Some(_) => thread::sleep(Duration::from_millis(250)),
            }
        }
    }

    fn remove_widget(&self) -> Result<Step> {
        let dir = self.plugin_dir();
        let mut taken_out = false;
        match shell_plugins() {
            Some(plugins) if plugin_enabled(&plugins, PLUGIN_ID) => {
                taken_out = shell(&["setPluginEnabled", PLUGIN_ID, "false"]).as_deref() == Some("ok");
            }
            Some(_) => {}
            // Without the shell, the bar's layout gets edited directly.
            None => taken_out = drop_from_shell_config(&self.config().join("shell.json"))?,
        }
        if !dir.exists() {
            return Ok(removed(taken_out, "the widget"));
        }
        fs::remove_dir_all(&dir).with_context(|| format!("Couldn't remove {}", dir.display()))?;
        shell(&["rescanPlugins"]);
        Ok(Step::Done("taken out of the bar and removed".into()))
    }

    fn install_theme(&self) -> Result<Step> {
        write_if_changed(&self.template(), theme::TEMPLATE)?;
        let name = self.theme_name();
        if self.theme_ships_colors(&name) {
            return Ok(Step::Done(format!("follows the theme; {name} brings colors for cloofy")));
        }
        // Rendered now, so the colors don't have to wait for the next theme switch.
        let colors = self.state.join("current/theme/colors.toml");
        let variables = omarchy_variables(&colors)
            .unwrap_or_else(|| theme::variables(&fs::read_to_string(&colors).unwrap_or_default()));
        write_if_changed(&self.rendered(), &theme::render(theme::TEMPLATE, &variables))?;
        Ok(Step::Done(format!("follows the theme, now {name}")))
    }

    fn remove_theme(&self) -> Result<Step> {
        let had_template = remove_file(&self.template())?;
        // The theme's own file stays; the one rendered from the template goes with it.
        let rendered = !self.theme_ships_colors(&self.theme_name()) && remove_file(&self.rendered())?;
        Ok(removed(had_template || rendered, "the template"))
    }

    fn install_menu(&self) -> Result<Step> {
        let path = self.menu();
        let current = match fs::read_to_string(&path) {
            Ok(text) => Some(text),
            Err(err) if err.kind() == io::ErrorKind::NotFound => None,
            Err(err) => return Err(err).with_context(|| format!("Couldn't read {}", path.display())),
        };
        write_if_changed(&path, &add_menu_entries(current.as_deref()).with_context(|| left_alone(&path))?)?;
        Ok(Step::Done("Music in the Omarchy menu".into()))
    }

    fn install_launcher(&self) -> Result<Step> {
        let entry = self.desktop_entry();
        if entry.exists() && !is_ours(&entry) {
            return Ok(Step::Nothing(format!("{} is your own, so it stays", entry.display())));
        }
        write_if_changed(&self.icon(), ICON)?;
        write_if_changed(&entry, DESKTOP_ENTRY)?;
        let _ = Command::new("gtk-update-icon-cache").arg(self.home.join(".local/share/icons/hicolor")).output();
        Ok(Step::Done("cloofy in the app launcher".into()))
    }

    fn remove_launcher(&self) -> Result<Step> {
        if !is_ours(&self.desktop_entry()) {
            return Ok(removed(false, "the launcher"));
        }
        remove_file(&self.desktop_entry())?;
        if fs::read_to_string(self.icon()).is_ok_and(|icon| icon == ICON) {
            remove_file(&self.icon())?;
            // The folders `install` may have made for it, where nothing else is in them.
            for dir in self.icon().ancestors().skip(1).take(2) {
                let _ = fs::remove_dir(dir);
            }
            let _ = Command::new("gtk-update-icon-cache").arg(self.home.join(".local/share/icons/hicolor")).output();
        }
        Ok(Step::Done("removed".into()))
    }

    fn install_binding(&self) -> Result<Step> {
        let path = self.bindings();
        let current = fs::read_to_string(&path).with_context(|| format!("Couldn't read {}", path.display()))?;
        let bound = add_block(&current, "--", BINDING).with_context(|| left_alone(&path))?;
        if bound == current {
            return Ok(Step::Done("opens cloofy instead of Spotify".into()));
        }
        write_if_changed(&path, &bound)?;
        // Hyprland would read the file again by itself; reloading now is to hear about a mistake.
        let _ = Command::new("hyprctl").arg("reload").output();
        let errors = Command::new("hyprctl").arg("configerrors").output();
        if let Ok(output) = errors
            && !String::from_utf8_lossy(&output.stdout).trim().is_empty()
        {
            write_if_changed(&path, &current)?;
            let _ = Command::new("hyprctl").arg("reload").output();
            anyhow::bail!("Hyprland didn't take the binding, so it's left as it was");
        }
        Ok(Step::Done("opens cloofy instead of Spotify".into()))
    }

    fn remove_binding(&self) -> Result<Step> {
        remove_block_from(&self.bindings(), "--").map(|done| removed(done, "the binding"))
    }

    fn theme_name(&self) -> String {
        fs::read_to_string(self.state.join("current/theme.name")).map(|name| name.trim().to_owned()).unwrap_or_default()
    }

    /// Whether the theme comes with a `cloofy.toml` of its own, which Omarchy keeps over the
    /// template's.
    fn theme_ships_colors(&self, name: &str) -> bool {
        !name.is_empty()
            && [self.config().join("themes"), self.path.join("themes")]
                .iter()
                .any(|themes| themes.join(name).join("cloofy.toml").is_file())
    }
}

/// Asks before taking Super+Shift+M from Spotify; no answer without a terminal.
fn ask_music_key() -> bool {
    if !io::stdin().is_terminal() {
        return false;
    }
    print!("\n  Omarchy opens Spotify with {}. Make it open cloofy instead? [y/N] ", style::bold("Super+Shift+M"));
    let _ = io::stdout().flush();
    let mut answer = String::new();
    io::stdin().lock().read_line(&mut answer).is_ok() && matches!(answer.trim(), "y" | "Y" | "yes" | "Yes")
}

fn report(name: &str, result: Result<Step>) {
    match result {
        Ok(Step::Done(detail)) => ok(name, &detail),
        Ok(Step::Nothing(detail)) => skipped(name, &detail),
        Err(err) => println!("  {}  {name:<16}{}", style::red("✗"), style::red(&format!("{err:#}"))),
    }
}

fn ok(name: &str, detail: &str) {
    println!("  {}  {name:<16}{}", style::green("✓"), style::dim(detail));
}

fn skipped(name: &str, detail: &str) {
    println!("  {}", style::dim(&format!("–  {name:<16}{detail}")));
}

/// What `remove` says about something it took out, or found nothing of.
fn removed(done: bool, what: &str) -> Step {
    if done { Step::Done("removed".into()) } else { Step::Nothing(format!("{what} wasn't there")) }
}

/// Asks the running Omarchy shell; `None` without one.
fn shell(args: &[&str]) -> Option<String> {
    let output = Command::new("omarchy-shell").arg("shell").args(args).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn shell_plugins() -> Option<String> {
    shell(&["listPlugins"]).filter(|plugins| plugins.starts_with('['))
}

/// Whether the shell's plugin list has `id` in the bar.
fn plugin_enabled(plugins: &str, id: &str) -> bool {
    let Ok(serde_json::Value::Array(plugins)) = serde_json::from_str(plugins) else { return false };
    plugins.iter().any(|plugin| plugin["id"] == id && plugin["enabled"] == true)
}

/// Takes the widget out of `shell.json` while the shell isn't running, with jq, which Omarchy
/// edits it with too and which keeps the order of everything else. Returns whether it was there.
fn drop_from_shell_config(path: &Path) -> Result<bool> {
    let Ok(config) = fs::read_to_string(path) else { return Ok(false) };
    if !config.contains(PLUGIN_ID) {
        return Ok(false);
    }
    let filter = r#"if (.bar.layout | type) == "object" then .bar.layout |= map_values(if type == "array" then map(select((.id? // "") != $id)) else . end) else . end"#;
    let output = Command::new("jq")
        .args(["--indent", "2", "--arg", "id", PLUGIN_ID, filter])
        .arg(path)
        .output()
        .context("Couldn't run jq")?;
    anyhow::ensure!(output.status.success(), "jq: {}", String::from_utf8_lossy(&output.stderr).trim());
    write_if_changed(path, &String::from_utf8_lossy(&output.stdout))?;
    Ok(true)
}

/// The theme's variables the way Omarchy's templates see them, from Omarchy itself.
fn omarchy_variables(colors: &Path) -> Option<HashMap<String, String>> {
    let output = Command::new("omarchy-theme-color").arg("--file").arg(colors).arg("--all").output().ok()?;
    let variables: HashMap<_, _> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
    (output.status.success() && !variables.is_empty()).then_some(variables)
}

/// The menu file with cloofy's entries in it: a file of cloofy's own where there was none,
/// else the user's with the entries right after its opening brace, which keeps every comma
/// of theirs valid.
fn add_menu_entries(current: Option<&str>) -> Result<String> {
    let Some(current) = current else {
        return Ok(format!(
            "{{\n  // {BEGIN}. `cloofy omarchy remove` takes these out again; {CREATED}.\n{MENU_ENTRIES}  // {END}\n}}\n"
        ));
    };
    // A file cloofy created stays one, so that `remove` takes it away whole.
    let created = if current.lines().any(|line| line.contains(BEGIN) && line.contains(CREATED)) {
        format!("; {CREATED}")
    } else {
        String::new()
    };
    let current = remove_block(current, "//")?;
    let mut out = String::with_capacity(current.len() + MENU_ENTRIES.len() + 200);
    let mut placed = false;
    for line in current.split_inclusive('\n') {
        out.push_str(line);
        // Comments only ever take whole lines here, so the first brace outside one opens the
        // file's object.
        if !placed && !line.trim_start().starts_with("//") && line.contains('{') {
            let (before, after) = line.split_at(line.find('{').expect("contains a brace") + 1);
            out.truncate(out.len() - line.len());
            out.push_str(before);
            out.push('\n');
            out.push_str(&format!(
                "  // {BEGIN}. `cloofy omarchy remove` takes these out again{created}.\n{MENU_ENTRIES}  // {END}\n"
            ));
            let after = after.trim_start_matches([' ', '\t']);
            if !after.trim().is_empty() {
                out.push_str(after);
            }
            placed = true;
        }
    }
    if !placed {
        // Not an object to add to: start one.
        return add_menu_entries(None);
    }
    Ok(out)
}

/// `text` with cloofy's block at the end, in place of one it had.
fn add_block(text: &str, comment: &str, block: &str) -> Result<String> {
    let mut out = remove_block(text, comment)?;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!(
        "\n{comment} {BEGIN}. `cloofy omarchy remove` takes this out again.\n{block}{comment} {END}\n"
    ));
    Ok(out)
}

fn has_block(text: &str, comment: &str) -> bool {
    text.lines().any(|line| line.trim_start().starts_with(&format!("{comment} {BEGIN}")))
}

/// `text` without cloofy's block, and without the blank line `add_block` put before it. Where
/// the block's end line is gone, there is no telling where it ended, so that is an error.
fn remove_block(text: &str, comment: &str) -> Result<String> {
    let mut out = String::with_capacity(text.len());
    let mut inside = false;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if !inside && trimmed.starts_with(&format!("{comment} {BEGIN}")) {
            inside = true;
            if out.ends_with("\n\n") || out == "\n" {
                out.pop();
            }
            continue;
        }
        if inside {
            inside = !trimmed.starts_with(&format!("{comment} {END}"));
            continue;
        }
        out.push_str(line);
    }
    anyhow::ensure!(
        !inside,
        "“{comment} {BEGIN}” has no “{comment} {END}” after it. Add that line where cloofy's lines end, or delete them, and try again"
    );
    Ok(out)
}

/// What an error about a file says first: that the file stayed as it was.
fn left_alone(path: &Path) -> String {
    format!("Left {} as it was", path.display())
}

/// Takes cloofy's block out of the file at `path`, and the file too where cloofy created it.
/// Returns whether there was a block.
fn remove_block_from(path: &Path, comment: &str) -> Result<bool> {
    let Ok(text) = fs::read_to_string(path) else { return Ok(false) };
    if !has_block(&text, comment) {
        return Ok(false);
    }
    let created = text.lines().any(|line| line.contains(BEGIN) && line.contains(CREATED));
    let rest = remove_block(&text, comment).with_context(|| left_alone(path))?;
    if created && rest.chars().all(|c| c.is_whitespace() || c == '{' || c == '}') {
        remove_file(path)?;
    } else {
        write_if_changed(path, &rest)?;
    }
    Ok(true)
}

fn is_ours(desktop_entry: &Path) -> bool {
    fs::read_to_string(desktop_entry).is_ok_and(|entry| entry.lines().any(|line| line.trim() == DESKTOP_MARK))
}

/// Writes `content` unless the file has it already, through a temporary file so that a reader
/// never sees half of it. A symlink, as stow or chezmoi put in place, stays one: the file it
/// points to gets the content, and keeps its permissions.
fn write_if_changed(path: &Path, content: &str) -> Result<()> {
    if fs::read_to_string(path).is_ok_and(|current| current == content) {
        return Ok(());
    }
    let path = &link_target(path);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("Couldn't create {}", dir.display()))?;
    }
    let file_name = path.file_name().map(|name| name.to_string_lossy()).unwrap_or_default();
    let temporary = path.with_file_name(format!(".{file_name}.cloofy"));
    fs::write(&temporary, content).with_context(|| format!("Couldn't write {}", temporary.display()))?;
    if let Ok(metadata) = fs::metadata(path) {
        fs::set_permissions(&temporary, metadata.permissions())
            .with_context(|| format!("Couldn't write {}", temporary.display()))?;
    }
    fs::rename(&temporary, path).with_context(|| format!("Couldn't write {}", path.display()))
}

/// The file at the end of `path`'s symlinks, or `path` itself where it is none.
fn link_target(path: &Path) -> PathBuf {
    let mut path = path.to_path_buf();
    // As many as Linux follows.
    for _ in 0..40 {
        let Ok(target) = fs::read_link(&path) else { break };
        // A relative target starts from the link's folder; an absolute one replaces it all.
        path = path.parent().unwrap_or(Path::new("")).join(target);
    }
    path
}

/// Removes a file; whether there was one.
fn remove_file(path: &Path) -> Result<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err).with_context(|| format!("Couldn't remove {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER_MENU: &str = "{\n  // Extend the menu.\n  // \"personal\": {\"icon\":\"\",\"label\":\"Personal\"},\n  \"about\": {\"icon\":\"\",\"label\":\"About\"}\n}\n";

    /// The file as the shell reads it: whole-line comments out, trailing commas dropped.
    fn parse_menu(text: &str) -> serde_json::Value {
        let without_comments: String =
            text.lines().filter(|line| !line.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        let mut cleaned = String::new();
        let chars: Vec<char> = without_comments.chars().collect();
        for (i, c) in chars.iter().enumerate() {
            let next = chars[i + 1..].iter().find(|c| !c.is_whitespace());
            if *c == ',' && matches!(next, Some('}' | ']')) {
                continue;
            }
            cleaned.push(*c);
        }
        serde_json::from_str(&cleaned).expect("the menu file stays valid")
    }

    #[test]
    fn adds_menu_entries_without_touching_the_user_s() {
        let added = add_menu_entries(Some(USER_MENU)).unwrap();
        let menu = parse_menu(&added);
        assert_eq!(menu["about"]["label"], "About");
        assert_eq!(menu["music"]["label"], "Music");
        assert_eq!(menu["music.open"]["action"], "omarchy-launch-or-focus-tui cloofy");
        assert!(added.contains("// \"personal\""), "comments stay");
        assert_eq!(add_menu_entries(Some(&added)).unwrap(), added, "a second install changes nothing");
        assert_eq!(remove_block(&added, "//").unwrap(), USER_MENU, "remove gives back the file as it was");
    }

    #[test]
    fn creates_the_menu_file_and_takes_it_away_again() {
        let dir = env::temp_dir().join(format!("cloofy-omarchy-menu-{}", std::process::id()));
        let path = dir.join("omarchy-menu.jsonc");
        let created = add_menu_entries(None).unwrap();
        assert_eq!(parse_menu(&created)["music.next"]["label"], "Next");
        assert_eq!(add_menu_entries(Some(&created)).unwrap(), created, "installing again keeps it cloofy's");
        write_if_changed(&path, &created).unwrap();
        assert!(remove_block_from(&path, "//").unwrap());
        assert!(!path.exists(), "cloofy created it, so it goes");

        write_if_changed(&path, &add_menu_entries(Some("{}\n")).unwrap()).unwrap();
        assert!(remove_block_from(&path, "//").unwrap());
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\n}\n", "the user's file stays, if empty");
        assert!(!remove_block_from(&path, "//").unwrap(), "nothing left to remove");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn leaves_a_file_alone_whose_block_lost_its_end() {
        let dir = env::temp_dir().join(format!("cloofy-omarchy-no-end-{}", std::process::id()));
        let path = dir.join("omarchy-menu.jsonc");
        let added = add_menu_entries(Some(USER_MENU)).unwrap();
        let broken = added.replace("  // cloofy: end\n", "");
        assert!(remove_block(&broken, "//").is_err(), "no guessing where the block ends");
        assert!(add_menu_entries(Some(&broken)).is_err());
        assert!(add_block(&broken.replace("//", "--"), "--", BINDING).is_err());

        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, &broken).unwrap();
        let err = remove_block_from(&path, "//").unwrap_err();
        assert!(format!("{err:#}").contains(&path.display().to_string()), "says which file: {err:#}");
        assert_eq!(fs::read_to_string(&path).unwrap(), broken, "the user's entries and the brace stay");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn writes_through_a_symlink() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let dir = env::temp_dir().join(format!("cloofy-omarchy-symlink-{}", std::process::id()));
        let dotfiles = dir.join("dotfiles/hypr");
        fs::create_dir_all(&dotfiles).unwrap();
        fs::create_dir_all(dir.join("config")).unwrap();
        let real = dotfiles.join("bindings.lua");
        fs::write(&real, "-- mine\n").unwrap();
        fs::set_permissions(&real, fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.join("config/bindings.lua");
        symlink("../dotfiles/hypr/bindings.lua", &link).unwrap();

        write_if_changed(&link, "-- mine\n-- cloofy's\n").unwrap();
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "the link stays a link");
        assert_eq!(fs::read_to_string(&real).unwrap(), "-- mine\n-- cloofy's\n");
        assert_eq!(fs::metadata(&real).unwrap().permissions().mode() & 0o777, 0o600, "the mode stays");
        assert!(!dotfiles.join(".bindings.lua.cloofy").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn handles_a_menu_on_one_line() {
        let added = add_menu_entries(Some("{ \"about\": {\"label\":\"About\"} }\n")).unwrap();
        let menu = parse_menu(&added);
        assert_eq!(menu["about"]["label"], "About");
        assert_eq!(menu["music"]["icon"], "󰝚");
    }

    #[test]
    fn appends_the_binding_and_takes_it_out_exactly() {
        let bindings = "-- Keep only your personal keybinding overrides here.\n-- o.bind(\"SUPER + SHIFT + R\", \"SSH\", \"alacritty -e ssh\")\n";
        let added = add_block(bindings, "--", BINDING).unwrap();
        assert!(added.starts_with(bindings));
        assert!(added.contains("hl.unbind(\"SUPER + SHIFT + M\")\no.bind(\"SUPER + SHIFT + M\", \"Music\", { tui = \"cloofy\", focus = true })\n-- cloofy: end\n"));
        assert_eq!(add_block(&added, "--", BINDING).unwrap(), added, "only once");
        assert_eq!(remove_block(&added, "--").unwrap(), bindings);
        let no_newline = "o.bind(\"SUPER + X\", nil, \"x\")";
        assert_eq!(
            remove_block(&add_block(no_newline, "--", BINDING).unwrap(), "--").unwrap(),
            format!("{no_newline}\n")
        );
    }

    #[test]
    fn knows_its_own_desktop_entry() {
        let dir = env::temp_dir().join(format!("cloofy-omarchy-desktop-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let ours = dir.join("ours.desktop");
        let theirs = dir.join("theirs.desktop");
        fs::write(&ours, DESKTOP_ENTRY).unwrap();
        fs::write(&theirs, "[Desktop Entry]\nName=cloofy\nExec=kitty -e cloofy\n").unwrap();
        assert!(is_ours(&ours));
        assert!(!is_ours(&theirs));
        assert!(!is_ours(&dir.join("missing.desktop")));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reads_the_shell_s_plugin_list() {
        let plugins = r#"[{"id":"omarchy.clock","enabled":true},{"id":"cloofy.nowplaying","enabled":false}]"#;
        assert!(plugin_enabled(plugins, "omarchy.clock"));
        assert!(!plugin_enabled(plugins, PLUGIN_ID));
        assert!(!plugin_enabled("not json", PLUGIN_ID));
    }

    #[test]
    fn ships_a_widget_omarchy_accepts() {
        let manifest: serde_json::Value = serde_json::from_str(PLUGIN[0].1).unwrap();
        assert_eq!(manifest["id"], PLUGIN_ID);
        assert_eq!(manifest["schemaVersion"], 1);
        assert_eq!(manifest["entryPoints"]["barWidget"], "Widget.qml");
        assert!(PLUGIN.iter().any(|(name, _)| *name == "Widget.qml"));
    }
}
