//! The browser extension's offer: at each start, until a browser's extension has reached the
//! app (the native host notes it: mdm_ipc::extension::seen), a window lists the browsers
//! installed and opens the extension's page in the one picked (addons.mozilla.org for Firefox
//! and its family, the Chrome Web Store for Chrome, Edge, Brave, Opera and Vivaldi), with the
//! tutorial that adds it by hand as the fallback. Chrome may ask by itself too: registering the
//! host also names the store's extension to it (crates/nmhost). "Don't ask again" stops the
//! offer; `/show extension` brings it back.
//!
//! Safari (macOS) has its extension in the app already (tools/safari): its row says "How to
//! Add", which opens the steps that turn on an extension Apple hasn't notarized.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use egui::{Pos2, Ui};

use crate::dialogs::{Outcome, window};
use crate::winui::{self, ButtonKind, r};

const W: f32 = 520.0;
const W_GUIDE: f32 = 560.0;
const BANNER: f32 = 84.0;
const ROW: f32 = 34.0;
/// Safari's row (macOS), as Options and the native host name it.
const SAFARI: &str = "Apple Safari";

/// Minimum extension version that includes the fork's catch-all capture, Zalo `blob:` support,
/// and dedicated fork ID. Earlier versions (such as upstream 0.5.2) are treated as outdated.
pub const MIN_FORK_EXT_VERSION: (u32, u32, u32) = (0, 5, 8);

/// Connection state of the browser extension seen by the native messaging host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtState {
    /// At least one browser connected with an up-to-date fork extension (`>= 0.5.8`).
    Connected,
    /// Only older extension version(s) have connected (holds the highest seen version string).
    Outdated(String),
    /// No browser extension has connected yet.
    Missing,
}

pub fn is_vi() -> bool {
    crate::i18n::code().starts_with("vi")
}

pub fn bi<'a>(vi: &'a str, en: &'a str) -> &'a str {
    if is_vi() { vi } else { en }
}

/// High-contrast amber warning color adapted to light and dark surfaces.
pub fn warn_color() -> egui::Color32 {
    if crate::theme::dark_surface() {
        egui::Color32::from_rgb(245, 158, 11)
    } else {
        egui::Color32::from_rgb(180, 83, 9)
    }
}

fn parse_semver(v: &str) -> Option<(u32, u32, u32)> {
    let v = v.trim().strip_prefix('v').unwrap_or(v.trim());
    let mut parts = v.split('.');
    let maj = parts.next()?.parse().ok()?;
    let min = parts.next()?.parse().ok()?;
    let patch_raw = parts.next().unwrap_or("0");
    let patch_digits: String = patch_raw
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let patch = if patch_digits.is_empty() {
        0
    } else {
        patch_digits.parse().ok()?
    };
    Some((maj, min, patch))
}

pub fn version_at_least(v: &str, min: (u32, u32, u32)) -> bool {
    parse_semver(v).is_some_and(|parsed| parsed >= min)
}

pub fn state_from_seen(seen: &[(String, String)]) -> ExtState {
    if seen.is_empty() {
        return ExtState::Missing;
    }
    if seen
        .iter()
        .any(|(_, v)| version_at_least(v, MIN_FORK_EXT_VERSION))
    {
        return ExtState::Connected;
    }
    let best = seen
        .iter()
        .filter(|(_, v)| !v.trim().is_empty())
        .max_by_key(|(_, v)| parse_semver(v))
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    ExtState::Outdated(best)
}

/// Current browser extension status read from the native host's `extension-seen` file.
pub fn state() -> ExtState {
    state_from_seen(&mdm_ipc::extension::seen_browsers())
}

fn detected_browsers() -> Vec<(String, Option<PathBuf>)> {
    #[cfg(windows)]
    let found = crate::win32::browsers();
    #[cfg(unix)]
    let found = crate::unix::browsers();
    let mut browsers: Vec<_> = found.into_iter().map(|(n, p)| (n, Some(p))).collect();
    // The table's order (Chrome first, its button the accent one), others after.
    browsers.sort_by_key(|(n, _)| {
        mdm_ipc::browsers::BROWSERS
            .iter()
            .position(|b| b.name == n.as_str())
            .unwrap_or(usize::MAX)
    });
    // Safari comes with the app (macOS only): a row of its own, after the others.
    if safari_here() {
        browsers.push((SAFARI.to_owned(), None));
    }
    if browsers.is_empty() {
        browsers.push((String::new(), None));
    }
    browsers
}

/// Help menu label for the in-app quick usage guide.
pub fn guide_menu_label() -> &'static str {
    bi("&Hướng dẫn sử dụng...", "&Usage guide...")
}

/// Help menu label for opening the browser extension setup dialog anytime.
pub fn extension_menu_label() -> &'static str {
    bi("&Cài tiện ích trình duyệt...", "&Browser extension...")
}

/// The extensions settings URL for a Chromium-family browser.
pub fn extensions_url(browser: &str) -> &'static str {
    let lower = browser.to_ascii_lowercase();
    if lower.contains("edge") {
        "edge://extensions"
    } else if lower.contains("brave") {
        "brave://extensions"
    } else if lower.contains("opera") {
        "opera://extensions"
    } else if lower.contains("vivaldi") {
        "vivaldi://extensions"
    } else {
        "chrome://extensions"
    }
}

/// Bundled Chromium extension folder beside the running executable (when present) and the path
/// shown to the user in the in-app guide.
fn chromium_extension_location() -> (Option<PathBuf>, String) {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf));
    if let Some(dir) = exe_dir.map(|d| d.join("extension").join("chromium")) {
        let display = dir.display().to_string();
        let existing = dir.is_dir().then_some(dir);
        return (existing, display);
    }
    let fallback = if cfg!(windows) {
        r"extension\chromium".to_owned()
    } else {
        "extension/chromium".to_owned()
    };
    (None, fallback)
}

fn open_browser(program: &std::path::Path, url: &str) {
    #[cfg(windows)]
    crate::win32::open_in(program, url);
    #[cfg(unix)]
    crate::unix::open_in(program, url);
}

/// Whether to offer the extension at this start: browser integration on and registered (not a
/// portable copy, which registers nothing), no current fork extension (`>= 0.5.8`) seen yet,
/// not refused.
pub fn worth_asking(settings: &crate::settings::Settings) -> bool {
    !settings.extension_never_ask
        && settings.advanced_integration
        && mdm_ipc::portable_dir().is_none()
        && state() != ExtState::Connected
}

pub struct Extension {
    /// The browsers installed: name and program. Empty: one row for the system's browser.
    browsers: Vec<(String, Option<PathBuf>)>,
    /// A browser's extension has reached the app since the window opened.
    connected: bool,
    /// The browsers whose extension has (Options' names, mdm_ipc::extension::seen_browsers).
    seen: Vec<String>,
    /// The browsers can find the app (the native host is registered).
    registered: bool,
    /// When the status files were read last (once a second, not every frame).
    looked: std::time::Instant,
    /// Set it up's runs seen finished ([`set_up_finished`]).
    set_up_seen: u64,
    /// The steps that add Safari's extension, while that window is open.
    steps: Option<SafariSteps>,
    /// The in-app step-by-step setup guide for a Chromium browser, while open.
    chromium_steps: Option<ChromiumSteps>,
}

/// Whether `row` (a browser as the window lists it) is among the browsers seen (as Options
/// names them). Vivaldi, Chromium, Helium, Thorium and Arc say they are Chrome (no brand of
/// their own, or Chrome's on purpose): they share Chrome's status. Firefox's forks give their
/// own name (`capture::browser_named`).
fn is_seen(seen: &[String], row: &str) -> bool {
    let named = match row {
        "Brave" => "Brave Browser",
        "Vivaldi" | "Chromium" | "Helium" | "Thorium" | "Arc" => "Google Chrome",
        other => other,
    };
    seen.iter().any(|b| b == named)
}

/// A row for a browser of Firefox's family (Firefox, Zen, LibreWolf …): it gets the add-on's
/// page on addons.mozilla.org, not the Chrome Web Store.
fn is_gecko(row: &str) -> bool {
    mdm_ipc::browsers::named(row).is_some_and(|b| b.family == mdm_ipc::browsers::Family::Gecko)
}

enum OpenSteps {
    Safari,
    Chromium(String, Option<PathBuf>),
}

impl Extension {
    pub fn new() -> Self {
        Extension {
            browsers: detected_browsers(),
            connected: state() == ExtState::Connected,
            seen: seen_names(),
            registered: registered(),
            looked: std::time::Instant::now(),
            set_up_seen: 0,
            steps: None,
            chromium_steps: None,
        }
    }

    /// The Safari steps, open at once (`/show safari-steps`: screenshots).
    pub fn open_safari_steps(&mut self) {
        self.steps = Some(SafariSteps);
    }

    /// The Chromium steps, open at once (`/show chromium-steps`).
    pub fn open_chromium_steps(&mut self) {
        let (name, program) = self.first_chromium_browser();
        self.chromium_steps = Some(ChromiumSteps::new(name, program, false));
    }

    fn first_chromium_browser(&self) -> (String, Option<PathBuf>) {
        self.browsers
            .iter()
            .find(|(n, _)| n != SAFARI && !is_gecko(n))
            .cloned()
            .unwrap_or_else(|| ("Google Chrome".to_owned(), None))
    }

    /// Open until closed; Done when "Don't ask again" was pressed.
    pub fn show(&mut self, ctx: &egui::Context) -> Outcome<()> {
        // The host marks the extension seen when a browser starts it: look again now and then
        // (each browser's own, as each adds it).
        if self.looked.elapsed() >= Duration::from_secs(1) || set_up_finished(&mut self.set_up_seen)
        {
            self.looked = std::time::Instant::now();
            self.connected = state() == ExtState::Connected;
            self.seen = seen_names();
            self.registered = registered();
        }
        ctx.request_repaint_after(Duration::from_secs(1));
        let h = BANNER + 16.0 + self.browsers.len() as f32 * ROW + 104.0;
        // A long translation's subline makes the banner taller (winui::banner).
        let extra = crate::dialogs::grow_extra(ctx, "extension-banner");
        let mut answer = None;
        let mut open_steps: Option<OpenSteps> = None;
        let outcome = window(
            ctx,
            "extension",
            // i18n: title of the window that offers the browser extension
            "Add Mister Download Manager to your browser",
            [W, h + extra],
            false,
            winui::BG(),
            |ui, o| {
                self.body(ui, o, h, &mut answer, &mut open_steps);
                match answer.take() {
                    Some(true) => Outcome::Done(()),
                    Some(false) => Outcome::Closed,
                    // Escape closes the steps first, when they are open.
                    None if self.steps.is_none()
                        && self.chromium_steps.is_none()
                        && winui::escape_pressed(ui) =>
                    {
                        Outcome::Closed
                    }
                    None => Outcome::Open,
                }
            },
        );
        match open_steps {
            Some(OpenSteps::Safari) if self.steps.is_none() => {
                self.steps = Some(SafariSteps);
            }
            Some(OpenSteps::Chromium(name, program)) => {
                if self.chromium_steps.is_some() {
                    crate::dialogs::raise(ctx, crate::dialogs::dialog_viewport("chromium-steps"));
                }
                self.chromium_steps = Some(ChromiumSteps::new(name, program, false));
            }
            _ => {}
        }
        if let Some(steps) = &mut self.steps
            && !matches!(steps.show(ctx), Outcome::Open)
        {
            self.steps = None;
        }
        if let Some(steps) = &mut self.chromium_steps
            && !matches!(steps.show(ctx), Outcome::Open)
        {
            self.chromium_steps = None;
        }
        outcome
    }

    fn body(
        &mut self,
        ui: &mut Ui,
        o: Pos2,
        h: f32,
        answer: &mut Option<bool>,
        open_steps: &mut Option<OpenSteps>,
    ) {
        let p = ui.painter().clone();
        let (ink, muted) = (winui::TEXT(), winui::MUTED());
        // The banner: the logo, what the extension does, in one line, and how.
        let extra = winui::banner(
            ui,
            &p,
            o,
            W,
            "extension-banner",
            // i18n: the headline of the window that offers the browser extension
            crate::i18n::t("Catch downloads from your browser"),
            crate::i18n::t(
                "Add the free extension: files you download in your browser come here, and videos get a Download button.",
            ),
        );
        // Everything below it moves down with a taller banner.
        let o = o + egui::vec2(0.0, extra);

        // One row per browser: Firefox's family opens addons.mozilla.org, Safari opens its
        // built-in steps, and Chromium browsers open the in-app fork extension guide.
        for (k, (name, program)) in self.browsers.iter().enumerate() {
            let y = BANNER + 16.0 + k as f32 * ROW;
            winui::text_font(
                &p,
                winui::tahoma_bold(),
                o.x + 22.0,
                o.y + y + 16.0,
                if name.is_empty() {
                    // i18n: a row of the browser extension's window when no browser was found by name
                    crate::i18n::t("Your browser")
                } else {
                    name.as_str()
                },
                ink,
            );
            let safari = name == SAFARI;
            let gecko = is_gecko(name);
            let label = if safari {
                // i18n: button in the browser extension's window, on Safari's row: open the steps that turn the extension on
                "How to Add"
            } else if gecko {
                // i18n: button in the browser extension's window: open the extension's page in that browser
                "Add the extension"
            } else {
                // i18n: button in the browser extension's window: open the tutorial that adds the extension
                "Show me how"
            };
            // As wide as its label in this language (#192).
            let (add_x, add_w) =
                winui::buttons_to_right(&p, W - 18.0, 8.0, &[(label, 150.0)], 220.0)[0];
            // This browser's extension is there already: said, before its button.
            if is_seen(&self.seen, name) {
                winui::text_right(
                    &p,
                    o.x + add_x - 10.0,
                    o.y + y + 16.0,
                    &format!(
                        "\u{2714} {}",
                        // i18n: in the browser extension's window, beside a browser whose extension reached MDM
                        crate::i18n::t("Connected")
                    ),
                    winui::SUCCESS(),
                );
            }
            let add = winui::button(
                ui,
                r(o, add_x, y, add_w, 25.0),
                label,
                if k == 0 {
                    ButtonKind::Accent
                } else {
                    ButtonKind::Normal
                },
                false,
                None,
            );
            if add.clicked() && safari {
                *open_steps = Some(OpenSteps::Safari);
            } else if add.clicked() && gecko {
                let url = mdm_ipc::extension::FIREFOX_PAGE;
                match program {
                    Some(program) => open_browser(program, url),
                    None => crate::launch::open(url),
                }
            } else if add.clicked() {
                *open_steps = Some(OpenSteps::Chromium(name.clone(), program.clone()));
            }
        }

        // Where it stands, then the answer. Not set up at all (the setup's box unticked): no
        // extension could reach the app, so that comes first, with the fix.
        let y = h - 84.0;
        if !self.registered {
            // As wide as its label in this language (#192); the paragraph ends 10 px before it.
            let (fix_x, fix_w) =
                winui::buttons_to_right(&p, W - 18.0, 8.0, &[("Set it up", 110.0)], 180.0)[0];
            winui::paragraph(
                &p,
                o.x + 18.0,
                o.y + y,
                crate::i18n::t(
                    "Browser integration isn't set up on this computer, so no browser can reach Mister Download Manager yet.",
                ),
                winui::ERROR(),
                o.x + fix_x - 10.0,
                14.0,
            );
            let fix = winui::button(
                ui,
                r(o, fix_x, y - 4.0, fix_w, 25.0),
                // i18n: button in the browser extension's window: register MDM's helper with the browsers
                "Set it up",
                ButtonKind::Accent,
                false,
                None,
            );
            if fix.clicked() {
                set_up(ui.ctx());
            }
        }
        let (line, color) = if !self.registered {
            (String::new(), muted)
        } else if self.connected {
            (
                crate::i18n::t("Connected: downloads from your browser now come here.").to_owned(),
                winui::SUCCESS(),
            )
        } else {
            (
                bi(
                    "Bấm \"Chỉ tôi cách làm\" cạnh trình duyệt để xem hướng dẫn 1 lần (~20 giây). Cài xong sẽ tự báo Đã kết nối.",
                    "Click \"Show me how\" beside your browser for the 1-time setup guide (~20s). Updates automatically once connected.",
                )
                .to_owned(),
                muted,
            )
        };
        if !line.is_empty() {
            winui::paragraph(&p, o.x + 18.0, o.y + y, &line, color, o.x + W - 18.0, 14.0);
        }
        winui::etched_h(&p, o.x + 16.0, o.y + h - 46.0, W - 32.0);
        let y = h - 36.0;
        // Direct link to open the step-by-step manual Chromium extension guide.
        if winui::link(
            ui,
            &p,
            o.x + 18.0,
            o.y + y + 17.0,
            // i18n: link in the browser extension's window: the tutorial that adds it without the browser's store
            "Install it by hand",
            "by-hand",
        ) {
            let (name, program) = self.first_chromium_browser();
            *open_steps = Some(OpenSteps::Chromium(name, program));
        }
        // As wide as their labels in this language (#192), Don't ask again only while no
        // extension has reached MDM.
        let row: Vec<(&str, f32)> = if self.connected {
            vec![("Close", 90.0)]
        } else {
            vec![("Don't ask again", 130.0), ("Close", 90.0)]
        };
        let placed = winui::buttons_to_right(&p, W - 18.0, 8.0, &row, 200.0);
        let close_at = placed[placed.len() - 1];
        let close = winui::button(
            ui,
            r(o, close_at.0, y, close_at.1, 25.0),
            "Close",
            ButtonKind::Default,
            false,
            None,
        );
        if close.clicked() || winui::enter_pressed(ui) {
            *answer = Some(false);
        }
        if !self.connected {
            // i18n: button in the browser extension's window: stop offering it at each start
            let never = winui::button(
                ui,
                r(o, placed[0].0, y, placed[0].1, 25.0),
                "Don't ask again",
                ButtonKind::Normal,
                false,
                None,
            );
            if never.clicked() {
                *answer = Some(true);
            }
        }
    }
}

/// Is Safari on this computer (macOS)? Its extension is in the app.
fn safari_here() -> bool {
    cfg!(target_os = "macos")
        && [
            "/Applications/Safari.app",
            "/System/Cryptexes/App/System/Applications/Safari.app",
        ]
        .iter()
        .any(|p| std::path::Path::new(p).exists())
}

/// The steps that turn Safari's extension on. It is in the app, but an app Apple has not
/// notarized (a build of our own, the experimental downloads) has Safari list its extension only
/// while it allows unsigned ones, which it forgets each time it quits.
pub struct SafariSteps;

impl SafariSteps {
    pub fn show(&mut self, ctx: &egui::Context) -> Outcome<()> {
        // The banner and the steps are taller than planned in a language that wraps them.
        let extra = crate::dialogs::grow_extra(ctx, "safari-banner")
            + crate::dialogs::grow_extra(ctx, "safari-steps");
        let h = BANNER + 18.0 + STEPS.len() as f32 * STEP + 10.0 + NOTE + 54.0;
        window(
            ctx,
            "safari-steps",
            // i18n: title of the window that shows how to turn on the Safari extension
            "Add the extension to Safari",
            [W, h + extra],
            false,
            winui::BG(),
            |ui, o| {
                let h = h + extra;
                let p = ui.painter().clone();
                let banner_extra = winui::banner(
                    ui,
                    &p,
                    o,
                    W,
                    "safari-banner",
                    // i18n: the headline of the window that shows how to turn on the Safari extension
                    crate::i18n::t("Turn on the extension in Safari"),
                    crate::i18n::t(
                        "The extension is already in this app. Apple hasn't notarized this build, so Safari needs these steps to allow it.",
                    ),
                );
                let o = o + egui::vec2(0.0, banner_extra);
                let (ink, muted) = (winui::TEXT(), winui::MUTED());
                let mut y = BANNER + 18.0;
                let mut lines_extra = 0.0;
                for (k, step) in STEPS.iter().enumerate() {
                    winui::text(
                        &p,
                        o.x + 22.0,
                        o.y + y + 12.0,
                        &format!("{}.", k + 1),
                        muted,
                    );
                    let lines = winui::paragraph(
                        &p,
                        o.x + 48.0,
                        o.y + y + 12.0,
                        crate::i18n::t(step),
                        ink,
                        o.x + W - 18.0,
                        14.0,
                    );
                    let more = 14.0 * (lines.max(1) as f32 - 1.0);
                    y += STEP + more;
                    lines_extra += more;
                }
                // Safari forgets the unsigned setting when it quits.
                let note = winui::paragraph(
                    &p,
                    o.x + 22.0,
                    o.y + y + 14.0,
                    crate::i18n::t(
                        "Safari turns \"Allow unsigned extensions\" off each time it quits: do steps 1 to 6 again after restarting it.",
                    ),
                    muted,
                    o.x + W - 18.0,
                    14.0,
                );
                // NOTE plans two lines for it; a language that wraps it more grows the window.
                lines_extra += 14.0 * (note.max(2) as f32 - 2.0);
                crate::dialogs::grow_px(ui.ctx(), "safari-steps", lines_extra);
                // `o` moved down with the banner, which `h` already counts: the bottom row is
                // placed from the window's own top.
                winui::etched_h(&p, o.x + 16.0, o.y + h - 46.0 - banner_extra, W - 32.0);
                let placed = winui::buttons_to_right(&p, W - 18.0, 8.0, &[("Close", 90.0)], 200.0);
                let close = winui::button(
                    ui,
                    r(o, placed[0].0, h - 36.0 - banner_extra, placed[0].1, 25.0),
                    "Close",
                    ButtonKind::Default,
                    false,
                    None,
                );
                if close.clicked() || winui::enter_pressed(ui) || winui::escape_pressed(ui) {
                    Outcome::Closed
                } else {
                    Outcome::Open
                }
            },
        )
    }
}

/// A step's height when it fits one line, and the note's under them.
const STEP: f32 = 24.0;
const NOTE: f32 = 34.0;

/// The steps, in Safari's own words for its tabs and switches.
const STEPS: [&str; 8] = [
    // i18n: step 1 of turning on the Safari extension
    "Open Safari.",
    // i18n: step 2 of turning on the Safari extension; Cmd is the Command key
    "Open Safari's settings (Cmd+,).",
    // i18n: step 3 of turning on the Safari extension; the quoted tab name is what Safari's settings show in this language: use Apple's own word for it, or keep the English name when unsure
    "Switch to the \"Advanced\" tab (the cogwheels).",
    // i18n: step 4 of turning on the Safari extension; the quoted switch name is what Safari's settings show in this language: use Apple's own words for it, or keep the English name when unsure
    "Turn on \"Show features for web developers\".",
    // i18n: step 5 of turning on the Safari extension; the quoted tab name is what Safari's settings show in this language: use Apple's own word for it, or keep the English name when unsure
    "Switch to the \"Developer\" tab (the screwdriver and spanner).",
    // i18n: step 6 of turning on the Safari extension; the quoted switch name is what Safari's settings show in this language: use Apple's own words for it, or keep the English name when unsure
    "Turn on \"Allow unsigned extensions\".",
    // i18n: step 7 of turning on the Safari extension; the quoted tab name is what Safari's settings show in this language: use Apple's own word for it, or keep the English name when unsure
    "Switch to the \"Extensions\" tab (the puzzle piece).",
    // i18n: step 8 of turning on the Safari extension
    "Turn on \"Mister Download Manager\" and allow it on all websites.",
];

/// Action returned when `ChromiumSteps` closes with a follow-up request.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChromiumStepAction {
    /// Open the full browser picker so the user can choose a different browser (Edge, Firefox…).
    SwitchBrowser,
    /// User clicked "Don't ask again" on the standalone startup wizard.
    NeverAsk,
}

/// In-app step-by-step setup guide for loading the fork's bundled Chromium extension once,
/// with 1-click Copy / Open buttons and live connection detection.
pub struct ChromiumSteps {
    browser: String,
    program: Option<PathBuf>,
    standalone: bool,
    copied: Option<(String, std::time::Instant)>,
    ext_state: ExtState,
    looked: std::time::Instant,
}

impl ChromiumSteps {
    pub fn new(browser: String, program: Option<PathBuf>, standalone: bool) -> Self {
        Self {
            browser,
            program,
            standalone,
            copied: None,
            ext_state: state(),
            looked: std::time::Instant::now(),
        }
    }

    /// Open directly for the primary installed Chromium browser (e.g. from the main-window
    /// banner or status bar).
    pub fn for_primary_chromium() -> Self {
        let browsers = detected_browsers();
        let (name, program) = browsers
            .into_iter()
            .find(|(n, _)| n != SAFARI && !is_gecko(n))
            .unwrap_or_else(|| ("Google Chrome".to_owned(), None));
        Self::new(name, program, true)
    }

    /// On first launch / post-installer: jump straight to the 4-step guide when this computer
    /// only has Chromium-family browsers (no Firefox/Gecko and no Safari).
    pub fn for_chromium_only_system() -> Option<Self> {
        let browsers = detected_browsers();
        let has_non_chromium = browsers.iter().any(|(n, _)| n == SAFARI || is_gecko(n));
        if has_non_chromium {
            return None;
        }
        let (name, program) = browsers
            .into_iter()
            .next()
            .unwrap_or_else(|| ("Google Chrome".to_owned(), None));
        Some(Self::new(name, program, true))
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Outcome<ChromiumStepAction> {
        if self.looked.elapsed() >= Duration::from_secs(1) {
            self.looked = std::time::Instant::now();
            self.ext_state = state();
        }
        if self
            .copied
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() >= Duration::from_secs(4))
        {
            self.copied = None;
        }
        ctx.request_repaint_after(Duration::from_secs(1));

        let url = extensions_url(&self.browser);
        let display_browser = if self.browser.is_empty() {
            bi("trình duyệt", "your browser")
        } else {
            self.browser.as_str()
        };
        let title = if is_vi() {
            format!("Cài tiện ích cho {display_browser}")
        } else {
            format!("Add the extension to {display_browser}")
        };
        let (existing_dir, folder_str) = chromium_extension_location();
        let program = self.program.clone();
        let standalone = self.standalone;
        let ext_state = self.ext_state.clone();
        let connected = ext_state == ExtState::Connected;
        let copied_msg = self.copied.as_ref().map(|(m, _)| m.clone());
        let mut new_copied: Option<String> = None;

        let banner_extra = crate::dialogs::grow_extra(ctx, "chromium-banner");
        let body_extra = crate::dialogs::grow_extra(ctx, "chromium-body");
        let base_h = BANNER + 356.0;
        let total_h = base_h + banner_extra + body_extra;

        let out = window(
            ctx,
            "chromium-steps",
            &title,
            [W_GUIDE, total_h],
            false,
            winui::BG(),
            |ui, o| {
                let p = ui.painter().clone();
                let b_extra = winui::banner(
                    ui,
                    &p,
                    o,
                    W_GUIDE,
                    "chromium-banner",
                    &title,
                    bi(
                        "Chỉ cần làm 1 lần (~20 giây). Các bản cập nhật sau tự làm mới tiện ích, không cần cài lại.",
                        "One-time setup (~20 seconds). Future app updates refresh the extension folder automatically.",
                    ),
                );
                let o = o + egui::vec2(0.0, b_extra);
                let (ink, muted) = (winui::TEXT(), winui::MUTED());
                let mut y = BANNER + 16.0;

                // When an older upstream extension (e.g. 0.5.2) was previously seen, remind the
                // user to remove it in chrome://extensions before loading the new folder.
                if let ExtState::Outdated(ver) = &ext_state {
                    let note = if is_vi() {
                        if ver.is_empty() {
                            format!(
                                "Lưu ý: phát hiện tiện ích bản cũ — hãy bấm \"Xóa\" (Remove) bản cũ trong {url} trước khi nạp bản mới để tránh trùng 2 biểu tượng."
                            )
                        } else {
                            format!(
                                "Lưu ý: phát hiện tiện ích bản cũ ({ver}) — hãy bấm \"Xóa\" (Remove) bản cũ trong {url} trước khi nạp bản mới để tránh trùng 2 biểu tượng."
                            )
                        }
                    } else if ver.is_empty() {
                        format!(
                            "Note: an older extension was detected — click Remove on it in {url} before loading the new folder to avoid duplicate icons."
                        )
                    } else {
                        format!(
                            "Note: older extension ({ver}) detected — click Remove on it in {url} before loading the new folder to avoid duplicate icons."
                        )
                    };
                    let lines = winui::paragraph(
                        &p,
                        o.x + 20.0,
                        o.y + y + 12.0,
                        &note,
                        warn_color(),
                        o.x + W_GUIDE - 20.0,
                        14.0,
                    );
                    y += 14.0 * lines.max(1) as f32 + 10.0;
                }

                // Step 1: open browser extensions page.
                winui::text_font(
                    &p,
                    winui::tahoma_bold(),
                    o.x + 20.0,
                    o.y + y + 12.0,
                    "1.",
                    ink,
                );
                let step1 = if is_vi() {
                    format!(
                        "Mở trang tiện ích trình duyệt: dán {url} vào thanh địa chỉ rồi bấm Enter."
                    )
                } else {
                    format!(
                        "Open the extensions page: paste {url} into your browser's address bar and press Enter."
                    )
                };
                let lines = winui::paragraph(
                    &p,
                    o.x + 44.0,
                    o.y + y + 12.0,
                    &step1,
                    ink,
                    o.x + W_GUIDE - 20.0,
                    14.0,
                );
                y += 14.0 * lines.max(1) as f32 + 8.0;

                let copy_url_lbl = bi("Copy địa chỉ", "Copy address");
                let open_br_lbl = bi("Mở trình duyệt", "Open browser");
                if let Some(prog) = &program {
                    let btns = winui::buttons_from_left(
                        &p,
                        44.0,
                        8.0,
                        &[(copy_url_lbl, 108.0), (open_br_lbl, 118.0)],
                        240.0,
                    );
                    if winui::button(
                        ui,
                        r(o, btns[0].0, y, btns[0].1, 24.0),
                        copy_url_lbl,
                        ButtonKind::Accent,
                        false,
                        None,
                    )
                    .clicked()
                    {
                        crate::copy_text(ui.ctx(), url.to_owned());
                        new_copied = Some(if is_vi() {
                            format!(
                                "\u{2714} Đã sao chép {url} — dán (Ctrl+V) vào thanh địa chỉ trình duyệt!"
                            )
                        } else {
                            format!(
                                "\u{2714} Copied {url} — paste (Ctrl+V) into your browser's address bar!"
                            )
                        });
                    }
                    if winui::button(
                        ui,
                        r(o, btns[1].0, y, btns[1].1, 24.0),
                        open_br_lbl,
                        ButtonKind::Normal,
                        false,
                        None,
                    )
                    .clicked()
                    {
                        crate::copy_text(ui.ctx(), url.to_owned());
                        open_browser(prog, url);
                        new_copied = Some(if is_vi() {
                            format!(
                                "\u{2714} Đã mở trình duyệt và sao chép {url} (nếu chưa tự mở trang, hãy bấm Ctrl+V vào thanh địa chỉ)."
                            )
                        } else {
                            format!(
                                "\u{2714} Opened browser and copied {url} (paste Ctrl+V if it did not navigate automatically)."
                            )
                        });
                    }
                } else {
                    let btns =
                        winui::buttons_from_left(&p, 44.0, 8.0, &[(copy_url_lbl, 108.0)], 240.0);
                    if winui::button(
                        ui,
                        r(o, btns[0].0, y, btns[0].1, 24.0),
                        copy_url_lbl,
                        ButtonKind::Accent,
                        false,
                        None,
                    )
                    .clicked()
                    {
                        crate::copy_text(ui.ctx(), url.to_owned());
                        new_copied = Some(if is_vi() {
                            format!(
                                "\u{2714} Đã sao chép {url} — dán (Ctrl+V) vào thanh địa chỉ trình duyệt!"
                            )
                        } else {
                            format!(
                                "\u{2714} Copied {url} — paste (Ctrl+V) into your browser's address bar!"
                            )
                        });
                    }
                }
                y += 24.0 + 12.0;

                // Step 2: turn on Developer mode.
                winui::text_font(
                    &p,
                    winui::tahoma_bold(),
                    o.x + 20.0,
                    o.y + y + 12.0,
                    "2.",
                    ink,
                );
                let lines = winui::paragraph(
                    &p,
                    o.x + 44.0,
                    o.y + y + 12.0,
                    bi(
                        "Ở góc trên bên phải trang vừa mở, bật công tắc \"Chế độ cho nhà phát triển\" (Developer mode).",
                        "At the top-right corner of that page, turn on \"Developer mode\".",
                    ),
                    ink,
                    o.x + W_GUIDE - 20.0,
                    14.0,
                );
                y += 14.0 * lines.max(1) as f32 + 12.0;

                // Step 3: Load unpacked -> pick bundled extension folder.
                winui::text_font(
                    &p,
                    winui::tahoma_bold(),
                    o.x + 20.0,
                    o.y + y + 12.0,
                    "3.",
                    ink,
                );
                let lines = winui::paragraph(
                    &p,
                    o.x + 44.0,
                    o.y + y + 12.0,
                    bi(
                        "Bấm \"Tải tiện ích đã giải nén\" (Load unpacked) ở góc trên bên trái, rồi chọn thư mục sau:",
                        "Click \"Load unpacked\" at the top-left, then select this extension folder:",
                    ),
                    ink,
                    o.x + W_GUIDE - 20.0,
                    14.0,
                );
                y += 14.0 * lines.max(1) as f32 + 6.0;

                let box_rect = egui::Rect::from_min_size(
                    egui::pos2(o.x + 44.0, o.y + y),
                    egui::vec2(W_GUIDE - 64.0, 22.0),
                );
                winui::field_frame(&p, winui::rect(box_rect), winui::FieldLook::ReadOnly);
                let shown_path = winui::ellipsize_middle(
                    &p,
                    &winui::tahoma(),
                    &folder_str,
                    box_rect.width() - 12.0,
                );
                winui::text(
                    &p,
                    box_rect.left() + 6.0,
                    box_rect.top() + 15.0,
                    &shown_path,
                    ink,
                );
                if shown_path != folder_str {
                    winui::full_on_hover(
                        ui,
                        winui::rect(box_rect),
                        "chromium-ext-path",
                        &folder_str,
                    );
                }
                y += 22.0 + 8.0;

                let copy_dir_lbl = bi("Copy đường dẫn", "Copy folder path");
                let open_dir_lbl = bi("Mở thư mục tiện ích", "Open extension folder");
                let dir_btns = winui::buttons_from_left(
                    &p,
                    44.0,
                    8.0,
                    &[(copy_dir_lbl, 118.0), (open_dir_lbl, 146.0)],
                    260.0,
                );
                if winui::button(
                    ui,
                    r(o, dir_btns[0].0, y, dir_btns[0].1, 24.0),
                    copy_dir_lbl,
                    ButtonKind::Accent,
                    false,
                    None,
                )
                .clicked()
                {
                    crate::copy_text(ui.ctx(), folder_str.clone());
                    new_copied = Some(
                        bi(
                            "\u{2714} Đã sao chép đường dẫn — bấm Ctrl+V vào ô Folder khi trình duyệt hỏi chọn thư mục!",
                            "\u{2714} Copied folder path — press Ctrl+V in the browser's folder picker!",
                        )
                        .to_owned(),
                    );
                }
                if winui::button(
                    ui,
                    r(o, dir_btns[1].0, y, dir_btns[1].1, 24.0),
                    open_dir_lbl,
                    ButtonKind::Normal,
                    false,
                    None,
                )
                .clicked()
                {
                    if let Some(dir) = &existing_dir {
                        crate::launch::open(dir);
                    } else if let Some(parent) = std::env::current_exe()
                        .ok()
                        .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf))
                    {
                        crate::launch::open(parent);
                    } else {
                        crate::help::open_page(mdm_ipc::extension::GUIDE_DOC);
                    }
                }
                y += 24.0 + 12.0;

                // Step 4: ready to use + Alt+Click tip.
                winui::text_font(
                    &p,
                    winui::tahoma_bold(),
                    o.x + 20.0,
                    o.y + y + 12.0,
                    "4.",
                    ink,
                );
                let lines = winui::paragraph(
                    &p,
                    o.x + 44.0,
                    o.y + y + 12.0,
                    bi(
                        "Xong! Từ giờ mọi tệp tải trên trình duyệt (Zalo Web, PDF, Word, ảnh, nén...) tự động vào MDM. Giữ phím Alt + Click để tải bằng trình duyệt khi cần.",
                        "Done! Browser downloads (Zalo Web, PDF, Word, images, archives...) now go straight to MDM. Hold Alt + Click to download with the browser instead.",
                    ),
                    ink,
                    o.x + W_GUIDE - 20.0,
                    14.0,
                );
                y += 14.0 * lines.max(1) as f32 + 12.0;

                // Live connection status or clipboard feedback.
                let (status_text, status_color) = if connected {
                    (
                        bi(
                            "\u{2714} Đã kết nối! Tiện ích trình duyệt đang hoạt động — bạn có thể đóng cửa sổ này.",
                            "\u{2714} Connected! The browser extension is active — you can close this window.",
                        ),
                        winui::SUCCESS(),
                    )
                } else if let Some(msg) = copied_msg.as_deref() {
                    (msg, winui::SUCCESS())
                } else {
                    (
                        bi(
                            "Đang chờ trình duyệt kết nối... Ngay khi chọn xong thư mục ở bước 3, dòng này sẽ tự chuyển sang màu xanh.",
                            "Waiting for browser connection... As soon as you select the folder in step 3, this line turns green.",
                        ),
                        muted,
                    )
                };
                let lines = winui::paragraph(
                    &p,
                    o.x + 20.0,
                    o.y + y + 12.0,
                    status_text,
                    status_color,
                    o.x + W_GUIDE - 20.0,
                    14.0,
                );
                y += 14.0 * lines.max(1) as f32 + 10.0;

                crate::dialogs::grow_px(ui.ctx(), "chromium-body", (y + 52.0) - base_h);

                winui::etched_h(
                    &p,
                    o.x + 16.0,
                    o.y + total_h - 46.0 - b_extra,
                    W_GUIDE - 32.0,
                );
                let btn_y = total_h - 36.0 - b_extra;
                if standalone
                    && winui::link(
                        ui,
                        &p,
                        o.x + 18.0,
                        o.y + btn_y + 17.0,
                        bi("Chọn trình duyệt khác...", "Other browsers..."),
                        "switch-browser",
                    )
                {
                    return Outcome::Done(ChromiumStepAction::SwitchBrowser);
                }
                let row: Vec<(&str, f32)> = if standalone && !connected {
                    vec![("Don't ask again", 130.0), ("Close", 90.0)]
                } else {
                    vec![("Close", 90.0)]
                };
                let placed = winui::buttons_to_right(&p, W_GUIDE - 18.0, 8.0, &row, 200.0);
                let close_at = placed[placed.len() - 1];
                let close = winui::button(
                    ui,
                    r(o, close_at.0, btn_y, close_at.1, 25.0),
                    "Close",
                    ButtonKind::Default,
                    false,
                    None,
                );
                if standalone && !connected {
                    let never = winui::button(
                        ui,
                        r(o, placed[0].0, btn_y, placed[0].1, 25.0),
                        "Don't ask again",
                        ButtonKind::Normal,
                        false,
                        None,
                    );
                    if never.clicked() {
                        return Outcome::Done(ChromiumStepAction::NeverAsk);
                    }
                }
                if close.clicked() || winui::enter_pressed(ui) || winui::escape_pressed(ui) {
                    Outcome::Closed
                } else {
                    Outcome::Open
                }
            },
        );
        if let Some(msg) = new_copied {
            self.copied = Some((msg, std::time::Instant::now()));
        }
        out
    }
}

/// Sections shown in the built-in Usage Guide window (`Help → Usage guide...`):
/// `(vi_title, en_title, vi_body, en_body)`.
const GUIDE_SECTIONS: [(&str, &str, &str, &str); 5] = [
    (
        "1. Bắt tải tự động trên trình duyệt (Zalo Web, PDF, Word, ảnh...)",
        "1. Automatic browser capture (Zalo Web, PDF, Word, images...)",
        "Cài tiện ích trình duyệt 1 lần (bấm nút \"Cài tiện ích trình duyệt...\" bên dưới). Sau đó mọi tệp bạn tải trên Chrome, Edge, Brave, Firefox sẽ tự động chạy vào MDM. Khi muốn tải bằng trình duyệt, chỉ cần giữ phím Alt + Click.",
        "Install the browser extension once (click \"Browser extension...\" below). Downloads in Chrome, Edge, Brave and Firefox then go straight into MDM. Hold Alt + Click anytime to download with the browser instead.",
    ),
    (
        "2. Tải thủ công & bắt link từ Clipboard",
        "2. Manual downloads & clipboard capture",
        "Bấm \"+ Thêm URL\" (Ctrl+N) để dán link trực tiếp, hoặc chỉ cần copy (Ctrl+C) một đường link tải — MDM sẽ tự nhận và hỏi tải ngay. Tệp được chia nhiều luồng song song để đạt tốc độ cao nhất.",
        "Click \"+ Add URL\" (Ctrl+N) to paste a link directly, or copy (Ctrl+C) any download link and MDM offers to download it right away. Files split into parallel connections for maximum speed.",
    ),
    (
        "3. Tải Video & Torrent / Magnet",
        "3. Videos & Torrents / Magnet links",
        "Vào Tác vụ → Tải video từ trang web (hoặc bấm nút tải hiện trên video sau khi cài tiện ích) để tải video chất lượng cao. Bạn cũng có thể mở file .torrent hoặc dán link magnet trong menu Tệp.",
        "Open Tasks → Download video from a page (or use the Download button over web videos once the extension is installed). You can also open .torrent files or paste magnet links from the File menu.",
    ),
    (
        "4. Phân loại tệp, Hàng đợi & Lập lịch",
        "4. Categories, Queues & Scheduler",
        "Tệp tải xong tự sắp vào các mục Tài liệu, Nén, Chương trình, Nhạc, Video. Nhấp chuột phải vào tệp để mở, mở thư mục chứa tệp hoặc đổi tên; dùng Trình lập lịch để hẹn giờ tải hàng loạt.",
        "Finished files sort into Documents, Compressed, Programs, Music and Video. Right-click any download to open, reveal in folder, or rename; use the Scheduler to queue batch downloads.",
    ),
    (
        "5. Cập nhật tự động (OTA)",
        "5. Automatic updates (OTA)",
        "Vào Trợ giúp → Kiểm tra cập nhật... để lên phiên bản mới nhất. Bản cập nhật tự làm mới cả thư mục tiện ích (trên Chrome chỉ cần bấm tải lại tiện ích hoặc mở lại trình duyệt).",
        "Open Help → Check for updates... to install new releases. Updates also refresh the bundled extension folder automatically (in Chrome, click Reload or restart Chrome).",
    ),
];

/// In-app quick usage guide (`Help → Usage guide...`). Returns `Outcome::Done(true)` when the
/// user clicks the button to open the Browser Extension setup dialog.
pub struct UsageGuide;

impl UsageGuide {
    pub fn show(&mut self, ctx: &egui::Context) -> Outcome<bool> {
        let banner_extra = crate::dialogs::grow_extra(ctx, "guide-banner");
        let body_extra = crate::dialogs::grow_extra(ctx, "guide-body");
        let base_h = BANNER + 390.0;
        let total_h = base_h + banner_extra + body_extra;
        let title = bi("Hướng dẫn sử dụng", "Usage guide");

        window(
            ctx,
            "usage-guide",
            title,
            [W_GUIDE, total_h],
            false,
            winui::BG(),
            |ui, o| {
                let p = ui.painter().clone();
                let b_extra = winui::banner(
                    ui,
                    &p,
                    o,
                    W_GUIDE,
                    "guide-banner",
                    bi("Hướng dẫn sử dụng nhanh", "Quick usage guide"),
                    bi(
                        "Tóm tắt cách cài đặt và sử dụng Mister Download Manager ngay trong ứng dụng.",
                        "Everything you need to set up and use Mister Download Manager.",
                    ),
                );
                let o = o + egui::vec2(0.0, b_extra);
                let ink = winui::TEXT();
                let mut y = BANNER + 16.0;

                for (vi_h, en_h, vi_b, en_b) in GUIDE_SECTIONS {
                    winui::text_font(
                        &p,
                        winui::tahoma_bold(),
                        o.x + 20.0,
                        o.y + y + 12.0,
                        bi(vi_h, en_h),
                        ink,
                    );
                    y += 16.0;
                    let lines = winui::paragraph(
                        &p,
                        o.x + 20.0,
                        o.y + y + 12.0,
                        bi(vi_b, en_b),
                        ink,
                        o.x + W_GUIDE - 20.0,
                        14.0,
                    );
                    y += 14.0 * lines.max(1) as f32 + 12.0;
                }

                crate::dialogs::grow_px(ui.ctx(), "guide-body", (y + 52.0) - base_h);

                winui::etched_h(
                    &p,
                    o.x + 16.0,
                    o.y + total_h - 46.0 - b_extra,
                    W_GUIDE - 32.0,
                );
                let btn_y = total_h - 36.0 - b_extra;
                let ext_lbl = bi("Cài tiện ích trình duyệt...", "Browser extension...");
                let ext_at = winui::buttons_from_left(&p, 18.0, 8.0, &[(ext_lbl, 175.0)], 280.0)[0];
                let open_ext = winui::button(
                    ui,
                    r(o, ext_at.0, btn_y, ext_at.1, 25.0),
                    ext_lbl,
                    ButtonKind::Accent,
                    false,
                    None,
                );
                let close_at =
                    winui::buttons_to_right(&p, W_GUIDE - 18.0, 8.0, &[("Close", 90.0)], 160.0)[0];
                let close = winui::button(
                    ui,
                    r(o, close_at.0, btn_y, close_at.1, 25.0),
                    "Close",
                    ButtonKind::Default,
                    false,
                    None,
                );
                if open_ext.clicked() {
                    Outcome::Done(true)
                } else if close.clicked() || winui::enter_pressed(ui) || winui::escape_pressed(ui) {
                    Outcome::Closed
                } else {
                    Outcome::Open
                }
            },
        )
    }
}

/// The browsers whose current fork extension (`>= 0.5.8`) reached the app, by Options' names.
fn seen_names() -> Vec<String> {
    mdm_ipc::extension::seen_browsers()
        .into_iter()
        .filter(|(_, v)| version_at_least(v, MIN_FORK_EXT_VERSION))
        .map(|(b, _)| b)
        .collect()
}

/// Where browser integration's registration stands on this computer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Registration {
    /// The browsers find the native host's manifest, and the helper program it names is there.
    Ready,
    /// A browser's folder (or the registry) has no manifest.
    Missing,
    /// No browser MDM knows was found to register with (Linux and macOS find them by their
    /// folders; Windows registers for every browser at once).
    #[cfg_attr(windows, allow(dead_code))]
    NoBrowser,
    /// A manifest names a helper program that isn't there or can't be run (#253).
    HelperMissing(std::path::PathBuf),
}

/// Where browser integration's registration stands: the native host's manifests where the
/// browsers look, and the helper they name.
pub fn registration() -> Registration {
    #[cfg(windows)]
    return crate::win32::host_registration();
    #[cfg(unix)]
    return crate::unix::host_registration();
}

/// Is browser integration registered on this computer, ready for the browsers?
pub fn registered() -> bool {
    registration() == Registration::Ready
}

/// Why the last registration failed, in the system's words (Linux and macOS, where MDM
/// registers itself at each start).
fn register_error() -> Option<String> {
    #[cfg(windows)]
    return None;
    #[cfg(unix)]
    return crate::unix::register_error();
}

/// Set it up's runs finished so far: a window that shows the checks reads them again when this
/// changes ([`set_up_finished`]).
static SET_UP_DONE: AtomicU64 = AtomicU64::new(0);
/// A Set it up is running (a second press waits for it).
static SETTING_UP: AtomicBool = AtomicBool::new(false);
/// How long the native host's `--register` has (it writes a few files, or registry keys).
const SET_UP_LIMIT: Duration = Duration::from_secs(20);

/// Register it now, for a copy installed without it (the setup's box unticked): the native
/// host's own `--register`, for this user, on a thread of its own (a program waited for on the
/// window's thread freezes it for as long as the program takes, #222). The window repaints when
/// it's done, and [`set_up_finished`] tells the checks to read the result.
pub fn set_up(ctx: &egui::Context) {
    if SETTING_UP.swap(true, Ordering::AcqRel) {
        return;
    }
    let ctx = ctx.clone();
    let started = std::thread::Builder::new()
        .name("set-up".into())
        .spawn(move || {
            #[cfg(unix)]
            crate::unix::register_browsers_now(SET_UP_LIMIT);
            #[cfg(windows)]
            if let Ok(exe) = std::env::current_exe() {
                crate::launch::run_within(
                    std::process::Command::new(
                        exe.with_file_name("misterdownloadmanager-nmhost.exe"),
                    )
                    .arg("--register"),
                    SET_UP_LIMIT,
                );
            }
            SETTING_UP.store(false, Ordering::Release);
            SET_UP_DONE.fetch_add(1, Ordering::AcqRel);
            ctx.request_repaint();
        });
    if started.is_err() {
        SETTING_UP.store(false, Ordering::Release);
    }
}

/// Whether a Set it up finished since `seen` was last given here (it is then brought up to
/// date): the caller reads the checks again.
pub fn set_up_finished(seen: &mut u64) -> bool {
    let done = SET_UP_DONE.load(Ordering::Acquire);
    std::mem::replace(seen, done) != done
}

/// One line of Options → General → Test browser integration: what was checked, whether it
/// holds, and the help page's section when it doesn't.
pub struct Check {
    pub ok: bool,
    pub text: String,
    pub why: Option<&'static str>,
}

/// The checks in the order a connection needs them; the first failing one is what to fix.
pub fn checks(settings: &crate::settings::Settings) -> Vec<Check> {
    let check = |ok: bool, text: &str, why: Option<&'static str>| Check {
        ok,
        text: crate::i18n::t(text).to_owned(),
        why: (!ok).then_some(why).flatten(),
    };
    let mut out = vec![
        check(
            settings.advanced_integration,
            // i18n: a check of Test browser integration (Options → General)
            "Advanced browser integration is on",
            Some("options"),
        ),
        check(
            mdm_ipc::portable_dir().is_none(),
            // i18n: a check of Test browser integration: a portable copy can't be connected
            "This is the installed copy, not a portable one",
            Some("not-installed"),
        ),
    ];
    let registration = registration();
    out.push(match &registration {
        Registration::Ready | Registration::Missing => check(
            registration == Registration::Ready,
            // i18n: a check of Test browser integration: the browsers find MDM's helper program
            "The browsers can find Mister Download Manager",
            Some("not-installed"),
        ),
        Registration::NoBrowser => check(
            false,
            // i18n: a check of Test browser integration (Linux, macOS): MDM found no browser it can connect to (one installed as a Flatpak isn't found)
            "No browser MDM can connect to was found",
            Some("not-installed"),
        ),
        Registration::HelperMissing(path) => {
            tracing::warn!("browser integration: {} is missing", path.display());
            check(
                false,
                // i18n: a check of Test browser integration: the browsers are set up, but the small helper program they start (misterdownloadmanager-nmhost) is gone
                "The helper program the browsers start is missing",
                Some("not-installed"),
            )
        }
    });
    if registration != Registration::Ready
        && let Some(error) = register_error()
    {
        out.push(Check {
            ok: false,
            text: crate::i18n::fmt(
                // i18n: a check of Test browser integration: setting it up failed; {error} is the system's own message, in English
                crate::i18n::t("Setting it up failed: {error}"),
                &[("error", &error)],
            ),
            why: Some("not-installed"),
        });
    }
    let seen = mdm_ipc::extension::seen_browsers();
    if seen.is_empty() {
        out.push(check(
            false,
            // i18n: a check of Test browser integration: no browser's extension has reached MDM
            "No browser's extension has connected yet",
            None,
        ));
    }
    for (browser, version) in seen {
        let name = if browser.is_empty() {
            crate::i18n::t("Your browser").to_owned()
        } else {
            browser
        };
        let text = if version.is_empty() {
            crate::i18n::fmt(
                // i18n: a check of Test browser integration: {browser} is a browser's name
                crate::i18n::t("{browser}: the extension connected"),
                &[("browser", &name)],
            )
        } else {
            crate::i18n::fmt(
                // i18n: a check of Test browser integration: {browser} is a browser's name, {version} the extension's version number
                crate::i18n::t("{browser}: the extension {version} connected"),
                &[("browser", &name), ("version", &version)],
            )
        };
        out.push(Check {
            ok: true,
            text,
            why: None,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #41: Helium says it is Chrome; Zen gives its own name; each row reads its own status.
    #[test]
    fn rows_match_the_names_browsers_report() {
        let seen = vec!["Google Chrome".to_owned(), "Zen Browser".to_owned()];
        assert!(is_seen(&seen, "Helium") && is_seen(&seen, "Google Chrome"));
        assert!(is_seen(&seen, "Zen Browser"));
        assert!(!is_seen(&seen, "Mozilla Firefox") && !is_seen(&seen, "Brave"));
        assert!(is_seen(&["Brave Browser".to_owned()], "Brave"));
    }

    /// Safari's steps: a sentence each, all there, and Safari is named as the native host and
    /// Options name it (the row reads its status by that name).
    #[test]
    fn safari_steps_are_complete() {
        assert_eq!(STEPS.len(), 8);
        assert!(STEPS.iter().all(|s| s.ends_with('.')));
        assert!(is_seen(&[SAFARI.to_owned()], SAFARI));
        assert_eq!(SAFARI, "Apple Safari");
        assert!(
            mdm_ipc::capture::browser_of(
                "Mozilla/5.0 (Macintosh) AppleWebKit/605.1.15 Version/18.0 Safari/605.1.15"
            ) == SAFARI
        );
    }

    /// Firefox's family gets the tutorial, whatever the store has.
    #[test]
    fn gecko_rows_are_firefoxs_family() {
        for b in [
            "Mozilla Firefox",
            "Zen Browser",
            "LibreWolf",
            "Floorp",
            "Waterfox",
        ] {
            assert!(is_gecko(b), "{b}");
        }
        for b in ["Google Chrome", "Helium", "Brave", "Your browser", ""] {
            assert!(!is_gecko(b), "{b}");
        }
    }

    #[test]
    fn chromium_extensions_urls_match_each_browser() {
        assert_eq!(extensions_url("Google Chrome"), "chrome://extensions");
        assert_eq!(extensions_url("Microsoft Edge"), "edge://extensions");
        assert_eq!(extensions_url("Brave"), "brave://extensions");
        assert_eq!(extensions_url("Opera"), "opera://extensions");
        assert_eq!(extensions_url("Vivaldi"), "vivaldi://extensions");
        assert_eq!(extensions_url("Chromium"), "chrome://extensions");
        assert_eq!(extensions_url(""), "chrome://extensions");
    }

    #[test]
    fn usage_guide_sections_are_bilingual_and_non_empty() {
        assert_eq!(GUIDE_SECTIONS.len(), 5);
        for (vi_h, en_h, vi_b, en_b) in GUIDE_SECTIONS {
            assert!(!vi_h.is_empty() && !en_h.is_empty());
            assert!(!vi_b.is_empty() && !en_b.is_empty());
        }
    }

    #[test]
    fn ext_state_flags_missing_outdated_and_connected() {
        assert_eq!(state_from_seen(&[]), ExtState::Missing);
        assert_eq!(
            state_from_seen(&[("Google Chrome".into(), "0.5.2".into())]),
            ExtState::Outdated("0.5.2".into())
        );
        assert_eq!(
            state_from_seen(&[
                ("Google Chrome".into(), "0.5.2".into()),
                ("Microsoft Edge".into(), "0.5.7".into()),
            ]),
            ExtState::Outdated("0.5.7".into())
        );
        assert_eq!(
            state_from_seen(&[
                ("Google Chrome".into(), "0.5.2".into()),
                ("Google Chrome".into(), "0.5.8".into()),
            ]),
            ExtState::Connected
        );
        assert_eq!(
            state_from_seen(&[("Google Chrome".into(), "0.5.9".into())]),
            ExtState::Connected
        );
        assert!(version_at_least("0.5.8", MIN_FORK_EXT_VERSION));
        assert!(version_at_least("0.5.9", MIN_FORK_EXT_VERSION));
        assert!(version_at_least("1.0.0", MIN_FORK_EXT_VERSION));
        assert!(!version_at_least("0.5.2", MIN_FORK_EXT_VERSION));
        assert!(!version_at_least("0.5.7", MIN_FORK_EXT_VERSION));
        assert!(!version_at_least("", MIN_FORK_EXT_VERSION));
    }
}
