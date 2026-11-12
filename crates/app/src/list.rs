//! The download list, painted to IDM 6.42's pixels (notes/ui-spec/main-window.md "List view"),
//! with IDM's header behaviour: drag a header to reorder columns, drag a border to resize, click
//! to sort, right-click to choose columns.

use egui::{Color32, CursorIcon, Pos2, Rect, Sense, Ui, pos2};

use crate::chrome::{IconKey, ShellIcons};
use crate::scrollbar;
use crate::selection::Pick;
use crate::theme::themed;
// The list is laid out in English pixels; `fill` makes each one where it is on screen (the
// first column at the right in a right-to-left interface).
use crate::winui::{self, fill_l as fill, text_font};

/// The column headers' height (the theme's size; 24 in the classic look).
pub fn header_h() -> f32 {
    crate::theme::metrics().list_header
}
/// A row's height (the theme's size; 17 in the classic look).
pub fn row_h() -> f32 {
    crate::theme::metrics().list_row
}

// The active theme's colours (crate::palette, #45).
themed! {
    /// The list's border (it was #828790 in dark too): the lists' and tables' one edge.
    FRAME => list_frame;
    /// Inner ring and row background.
    BG => list;
    HEADER_BG => header;
    HEADER_SEP => header_sep;
    /// Sort indicator (an addition: IDM's header shows none).
    SORTED_BG => sorted;
    SORT_ARROW => sort_arrow;
    /// Light: the one grid colour, 1 px lines (light-main.md).
    GRID_H => list_grid;
    GRID_V_RIGHT => list_grid;
    /// Selected row (list focused): Windows' accent, as every list's (one blue across MDM), with
    /// white text and a dotted focus outline in its inverse, #ff9933.
    SELECTED => selection;
    TEXT => text;
    /// A filtered column's funnel.
    FILTER_ON => accent;
}

/// Columns, by their fixed data slot in [`Row::cells`]: IDM's eight, then the user's additions:
/// "Added" (2026-10-30: when the download was added; sorts in order of addition), "Source"
/// (D-014: a Web or Torrent chip, first by default), and a torrent's "Peers" and "Share ratio"
/// (#357: how many peers it has connected and what it has given back). "Upload speed" (#357: what
/// it sends) is the last slot: a saved layout is indexed by slot, so it must not take a slot an
/// older build already meant something else by.
pub const COLS: usize = 13;
pub const TITLES: [&str; COLS] = [
    "File Name",
    // i18n: column title: one letter for Queue (the column is narrow)
    "Q",
    // i18n: noun: the file's size
    "Size",
    // i18n: noun: the download's state or progress
    "Status",
    "Time left",
    "Transfer rate",
    "Last Try Date",
    // i18n: noun: the download's description
    "Description",
    // i18n: past participle: when the download was added
    "Added",
    // i18n: noun: where the download comes from (Web or Torrent)
    "Source",
    // i18n: column title: how many peers a torrent is connected to now (seeders and leechers)
    "Peers",
    // i18n: column title: what a torrent has sent to peers against what it took
    "Share ratio",
    // i18n: column title: what a torrent sends to peers, bytes per second
    "Upload speed",
];
/// The "Added" column's data slot.
pub const ADDED: usize = 8;
/// The "Source" column's data slot: its cell is "Web" or "Torrent", painted as a chip.
pub const SOURCE: usize = 9;
/// The "Peers" column's data slot (a torrent's connected peers; blank for anything else).
pub const PEERS: usize = 10;
/// The "Share ratio" column's data slot (a torrent's sent over taken; blank otherwise).
pub const RATIO: usize = 11;
/// The "Upload speed" column's data slot (what a torrent sends; blank for anything else). The
/// last slot, so a layout saved before it existed still means what it meant.
pub const UPLOAD: usize = 12;

/// IDM's default column widths (and ours for Added, room for a 12-hour clock's
/// `Oct 28 9:05:01 AM 2026`, Source, Peers, Share ratio and Upload speed).
pub const DEFAULT_WIDTHS: [f32; COLS] = [
    150.0, 20.0, 75.0, 65.0, 75.0, 95.0, 65.0, 200.0, 145.0, 62.0, 64.0, 92.0, 100.0,
];
/// Default display order: Source first, then IDM's, a torrent's three columns after its rate, and
/// Added after Last Try Date.
const DEFAULT_ORDER: [usize; COLS] = [SOURCE, 0, 1, 2, 3, 4, 5, UPLOAD, PEERS, RATIO, 6, ADDED, 7];

const MIN_WIDTH: f32 = 16.0;
/// How far from a separator a press grabs it for resizing.
const GRIP: f32 = 4.0;
/// Pointer travel before a header press becomes a column drag.
const DRAG_START: f32 = 5.0;

/// A column filter (beyond IDM, user 2026-10-29): a few fixed values to keep, or text the cell
/// must contain (case-insensitive).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Filter {
    Values(Vec<&'static str>),
    Text(String),
}

/// The values a column filters by, for columns that have a few (None: a text search).
pub fn filter_values(col: usize) -> Option<&'static [&'static str]> {
    match col {
        // i18n: where a download comes from: Web (web or FTP server) or Torrent (BitTorrent)
        SOURCE => Some(&["Web", "Torrent"]),
        1 => Some(&["In a queue", "Not in a queue"]),
        // i18n: a download's status (state it is in, not a command)
        3 => Some(&["Downloading", "Stopped", "Waiting", "Complete", "Error"]),
        _ => None,
    }
}

/// The value a row has for a value-list column.
fn row_value(row: &Row, col: usize) -> &str {
    match col {
        1 if row.queued => "In a queue",
        1 => "Not in a queue",
        3 => row.status_group,
        c => &row.cells[c],
    }
}

/// Does the row pass every column filter? A hidden column's filter is kept but doesn't filter
/// (nothing on screen would say why rows are missing).
pub fn matches(st: &ListState, row: &Row) -> bool {
    st.filters.iter().enumerate().all(|(c, f)| match f {
        _ if !st.visible[c] => true,
        None => true,
        Some(Filter::Values(keep)) => keep.contains(&row_value(row, c)),
        Some(Filter::Text(t)) => row.cells[c].to_lowercase().contains(&t.to_lowercase()),
    })
}

#[derive(Clone, Copy)]
enum HeaderDrag {
    /// Resizing column `col` (data slot) from `width` at pointer `x0`.
    Resize {
        col: usize,
        width: f32,
        x0: f32,
    },
    /// A press on column `col` at `x0`; becomes a move once it travels.
    Press {
        col: usize,
        x0: f32,
    },
    Move {
        col: usize,
        grab: f32,
    },
}

pub struct ListState {
    /// Display order: data slots, left to right.
    pub order: Vec<usize>,
    pub widths: [f32; COLS],
    pub visible: [bool; COLS],
    /// Sorted column (data slot) and ascending?
    pub sort: Option<(usize, bool)>,
    /// Vertical scroll in px (shown snapped to whole rows, as a Win32 list view scrolls).
    pub scroll: f32,
    /// Horizontal scroll in px.
    pub hscroll: f32,
    drag: Option<HeaderDrag>,
    /// The list has the keys: the selection in its colour (grey while it hasn't).
    focused: bool,
    /// Bring this row into view on the next frame (Find).
    pub scroll_to: Option<usize>,
    /// Column filters, by data slot (kept for the session).
    pub filters: Vec<Option<Filter>>,
    /// The open filter box: column and its top-left.
    filter_open: Option<(usize, Pos2)>,
    /// The sideways scroll when it opened (it closes when the list moves).
    filter_hscroll: f32,
    /// A selection box being dragged: where it started (x in content, y in rows' space) and
    /// whether Ctrl adds it to the selection.
    marquee: Option<(Pos2, bool)>,
    /// Type-ahead: the letters typed so far and when the last one came (seconds).
    typed: String,
    /// A menu is open: its keys (and letters) are its own, not the list's.
    pub menu_open: bool,
    typed_at: f64,
}

impl Default for ListState {
    fn default() -> Self {
        ListState {
            order: DEFAULT_ORDER.to_vec(),
            widths: DEFAULT_WIDTHS,
            visible: [true; COLS],
            sort: None,
            scroll: 0.0,
            hscroll: 0.0,
            drag: None,
            // The list has the keys from the start (as Explorer's does), not after a click.
            focused: true,
            scroll_to: None,
            filters: vec![None; COLS],
            filter_open: None,
            filter_hscroll: 0.0,
            marquee: None,
            typed: String::new(),
            menu_open: false,
            typed_at: 0.0,
        }
    }
}

/// The part of the column state that is saved with the download list.
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Layout {
    pub order: Vec<usize>,
    /// Lists (not fixed arrays), so layouts saved before a column was added still load.
    pub widths: Vec<f32>,
    pub visible: Vec<bool>,
    pub sort: Option<(usize, bool)>,
}

impl ListState {
    /// Does the list have the keys? (A click elsewhere, or Tab to the Categories tree,
    /// takes them.)
    pub fn has_keys(&self) -> bool {
        self.focused
    }

    /// Give the list the keys, or take them (the tree has them).
    pub fn set_keys(&mut self, on: bool) {
        self.focused = on;
    }

    /// Columns as they first were: order, widths, every one shown.
    pub fn reset_columns(&mut self) {
        let fresh = ListState::default();
        self.order = fresh.order;
        self.widths = fresh.widths;
        self.visible = fresh.visible;
        self.hscroll = 0.0;
    }

    pub fn layout(&self) -> Layout {
        Layout {
            order: self.order.clone(),
            widths: self.widths.to_vec(),
            visible: self.visible.to_vec(),
            sort: self.sort,
        }
    }

    /// Restore a saved layout. One saved before "Added" existed gets it after Last Try Date,
    /// one saved before "Source" gets it first, one saved before the torrent's columns gets them
    /// after Transfer rate, in this order: Upload speed, Peers, Share ratio. A layout that
    /// doesn't name every column exactly once is ignored.
    pub fn apply(&mut self, mut l: Layout) {
        if !l.order.contains(&SOURCE) {
            l.order.insert(0, SOURCE);
        }
        if !l.order.contains(&ADDED) {
            let at = l
                .order
                .iter()
                .position(|c| *c == 6)
                .map_or(l.order.len(), |p| p + 1);
            l.order.insert(at, ADDED);
        }
        if !l.order.contains(&PEERS) {
            let at = l
                .order
                .iter()
                .position(|c| *c == 5)
                .map_or(l.order.len(), |p| p + 1);
            l.order.insert(at, PEERS);
        }
        if !l.order.contains(&RATIO) {
            let at = l
                .order
                .iter()
                .position(|c| *c == PEERS)
                .map_or(l.order.len(), |p| p + 1);
            l.order.insert(at, RATIO);
        }
        // Last, and so before the two already there: a layout from before #357's second half
        // keeps Transfer rate, then Upload speed, Peers, Share ratio.
        if !l.order.contains(&UPLOAD) {
            let at = l
                .order
                .iter()
                .position(|c| *c == 5)
                .map_or(l.order.len(), |p| p + 1);
            l.order.insert(at, UPLOAD);
        }
        let mut seen = l.order.clone();
        seen.sort_unstable();
        if seen != (0..COLS).collect::<Vec<_>>() {
            return;
        }
        self.order = l.order;
        for (c, default) in DEFAULT_WIDTHS.iter().enumerate() {
            self.widths[c] = l.widths.get(c).copied().unwrap_or(*default).max(MIN_WIDTH);
            self.visible[c] = l.visible.get(c).copied().unwrap_or(true);
        }
        self.visible[0] = true;
        self.sort = l.sort.filter(|(c, _)| *c < COLS);
    }

    /// Visible columns in display order.
    fn shown(&self) -> Vec<usize> {
        self.order
            .iter()
            .copied()
            .filter(|c| self.visible[*c])
            .collect()
    }
}

/// Responsive column set and pixel widths for `avail_w`.
/// In classic mode (`!modern`), returns `st.shown()` and `st.widths` untouched.
/// In modern mode, collapses low-priority columns in compact windows (unless filtered)
/// and flexes `File Name` (slot 0) so active columns never truncate or overflow horizontally.
fn layout_for_width(st: &ListState, avail_w: f32, modern: bool) -> (Vec<usize>, [f32; COLS]) {
    let base_shown = st.shown();
    let mut w = st.widths;
    if !modern || avail_w <= 0.0 {
        return (base_shown, w);
    }
    let dragging_col = match st.drag {
        Some(HeaderDrag::Resize { col, .. }) => Some(col),
        _ => None,
    };
    let shown: Vec<usize> = base_shown
        .into_iter()
        .filter(|&c| {
            if c == 0 || st.filters[c].is_some() || dragging_col == Some(c) {
                return true;
            }
            if avail_w < 980.0 && matches!(c, ADDED | 6 | 7 | PEERS | RATIO | UPLOAD) {
                return false;
            }
            if avail_w < 820.0 && matches!(c, SOURCE | 1) {
                return false;
            }
            if avail_w < 660.0 && c == 4 {
                return false;
            }
            if avail_w < 540.0 && c == 5 {
                return false;
            }
            true
        })
        .collect();

    let roomy = avail_w >= 1100.0;
    let mins: [(usize, f32); 6] = [
        (2, if roomy { 148.0 } else { 134.0 }),
        (3, if roomy { 132.0 } else { 106.0 }),
        (4, if roomy { 125.0 } else { 110.0 }),
        (5, if roomy { 120.0 } else { 110.0 }),
        (ADDED, 155.0),
        (SOURCE, 62.0),
    ];
    for (col, min_w) in mins {
        if shown.contains(&col) && dragging_col != Some(col) && w[col] < min_w {
            w[col] = min_w;
        }
    }

    let other_sum: f32 = shown.iter().filter(|&&c| c != 0).map(|&c| w[c]).sum();
    let min_name_w = if avail_w >= 820.0 { 260.0 } else { 244.0 };
    if other_sum + min_name_w > avail_w {
        let mut trim = (other_sum + min_name_w) - avail_w;
        for (col, floor_w) in [
            (3usize, 104.0f32),
            (5, 106.0),
            (4, 106.0),
            (2, 132.0),
            (ADDED, 145.0),
            (SOURCE, 58.0),
        ] {
            if shown.contains(&col)
                && dragging_col != Some(col)
                && w[col] > floor_w
                && trim > 0.0
            {
                let cut = (w[col] - floor_w).min(trim);
                w[col] -= cut;
                trim -= cut;
            }
        }
    }

    let final_other: f32 = shown.iter().filter(|&&c| c != 0).map(|&c| w[c]).sum();
    if shown.contains(&0) && dragging_col != Some(0) {
        w[0] = (avail_w - final_other).round().max(150.0);
    }
    (shown, w)
}

pub struct Row {
    /// What the Status filter sees: Downloading, Stopped, Waiting, Complete or Error.
    pub status_group: &'static str,
    pub icon: IconKey,
    /// In a queue: the Q column shows a queue icon (IDM: stacked files).
    pub queued: bool,
    /// Known fraction 0.0..=1.0 for the wide FDM progress bar under File Name.
    pub progress: Option<f32>,
    /// Downloaded / total text in modern rows ("189,1 MB / 4,19 GB", or "63,07 MB" when complete).
    pub size_detail: String,
    /// Whether the inline row button can start or resume this download.
    pub can_resume: bool,
    pub cells: [String; COLS],
}

#[derive(Default)]
pub struct ListResponse {
    /// The rows' area (under the header, inside the scroll bars), English: where an empty
    /// list's message goes.
    pub body: Option<Rect>,
    /// How the selection changes (a click, a key, a selection box).
    pub pick: Option<Pick>,
    /// Inline quick-action button clicked on this row (Pause / Resume / Open folder).
    pub row_action: Option<usize>,
    /// Double-click on this row.
    pub double_clicked: Option<usize>,
    /// Enter on the focused row (Open, as its menu says, whatever a double-click does).
    pub enter: Option<usize>,
    /// Alt+Enter: the focused row's Properties.
    pub properties: Option<usize>,
    /// Right-click: the row (None = blank space) and where its menu opens.
    pub context: Option<(Option<usize>, Pos2)>,
    /// The header was right-clicked here (column chooser).
    pub header_context: Option<Pos2>,
    /// The sort column or direction changed.
    pub sort_changed: bool,
}

/// A Windows list view's keys while the list has the focus (and no text box does), with its
/// multiple selection. Up / Down, Page Up / Page Down (a page of rows) and Home / End move the
/// focus: alone they select that row, with Shift they select from the anchor to it, with Ctrl
/// they only move the focus, with Ctrl+Shift they add that range. Space selects the focused row,
/// Ctrl+Space puts it in or out, Ctrl+A selects all (also before the list was clicked). Enter is
/// a double-click on the focused row, Alt+Enter opens its Properties. Typing selects the next
/// file whose name starts with the letters typed (within a second of each other; one letter
/// again and again goes through the names starting with it). Returns true for Shift+F10 / the
/// Menu key: the focused row's context menu.
fn keyboard(
    ui: &Ui,
    st: &mut ListState,
    rows: &[Row],
    focus: Option<usize>,
    client_h: f32,
    out: &mut ListResponse,
) -> bool {
    let n = rows.len();
    if n == 0 || st.menu_open || ui.ctx().memory(|m| m.focused().is_some()) {
        return false;
    }
    if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::A)) {
        st.focused = true;
        out.pick = Some(Pick::All);
        return false;
    }
    if !st.focused {
        return false;
    }
    let page = ((client_h - row_h()) / row_h()).floor().max(1.0) as isize;
    // The first row in view (the list scrolls by whole rows).
    let top = (st.scroll / row_h()).round() as isize;
    let last = n as isize - 1;
    let cur = focus.map(|s| s as isize);
    let (to, mods) = ui.input(|i| {
        let k = |key| i.key_pressed(key) && !i.modifiers.alt;
        let to = if k(egui::Key::ArrowDown) {
            Some(cur.map_or(0, |c| c + 1))
        } else if k(egui::Key::ArrowUp) {
            Some(cur.map_or(0, |c| c - 1))
        // As Explorer: first to the last (first) row in view, then a page on.
        } else if k(egui::Key::PageDown) {
            let bottom = top + page - 1;
            Some(cur.map_or(0, |c| if c < bottom { bottom } else { c + page }))
        } else if k(egui::Key::PageUp) {
            Some(cur.map_or(0, |c| if c > top { top } else { c - page }))
        } else if k(egui::Key::Home) {
            Some(0)
        } else if k(egui::Key::End) {
            Some(last)
        } else {
            None
        };
        (to, i.modifiers)
    });
    if let Some(to) = to {
        let to = to.clamp(0, last) as usize;
        out.pick = Some(match (mods.command, mods.shift) {
            (false, false) => Pick::Only(to),
            (true, false) => Pick::Focus(to),
            (ctrl, true) => Pick::Range { to, add: ctrl },
        });
        st.scroll_to = Some(to);
        st.typed.clear();
        return false;
    }
    let (menu, enter, alt_enter, space) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::Menu)
                || (i.key_pressed(egui::Key::F10) && i.modifiers.shift_only()),
            i.key_pressed(egui::Key::Enter) && i.modifiers.is_none(),
            i.key_pressed(egui::Key::Enter) && i.modifiers.alt,
            i.key_pressed(egui::Key::Space),
        )
    });
    if let Some(f) = focus {
        if enter {
            out.enter = Some(f);
        } else if alt_enter {
            out.properties = Some(f);
        }
    }
    // Type-ahead (a Space while typing is part of the name).
    let now = ui.input(|i| i.time);
    if now - st.typed_at > 1.0 {
        st.typed.clear();
    }
    let typed: String = ui.input(|i| {
        if i.modifiers.command || i.modifiers.alt {
            return String::new();
        }
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect()
    });
    let typing = !typed.is_empty() && !(typed == " " && st.typed.is_empty());
    if typing {
        st.typed.push_str(&typed.to_lowercase());
        st.typed_at = now;
        let one_letter = st.typed.chars().all(|c| st.typed.starts_with(c));
        let (prefix, from) = if one_letter {
            // The same letter again: the next name starting with it.
            (
                st.typed.chars().take(1).collect::<String>(),
                focus.map_or(0, |f| f + 1),
            )
        } else {
            (st.typed.clone(), focus.unwrap_or(0))
        };
        if let Some(i) = (0..n)
            .map(|k| (from + k) % n)
            .find(|i| rows[*i].cells[0].to_lowercase().starts_with(&prefix))
        {
            out.pick = Some(Pick::Only(i));
            st.scroll_to = Some(i);
        }
    } else if space && let Some(f) = focus {
        out.pick = Some(match (mods.command, mods.shift) {
            (true, _) => Pick::Toggle(f),
            (false, true) => Pick::Range { to: f, add: false },
            (false, false) => Pick::Only(f),
        });
    }
    menu
}

/// Paint the list inside `outer` (its 1 px #828790 border rect). `selected`: each row's
/// selection; `focus`: the focused row (the dotted outline, where keys move from).
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    outer: Rect,
    st: &mut ListState,
    rows: &[Row],
    selected: &[bool],
    focus: Option<usize>,
    icons: &mut ShellIcons,
    // The Status cell's tip for row `i` (asked for the row under the pointer only).
    status_tip: &dyn Fn(usize) -> String,
) -> ListResponse {
    let mut out = ListResponse::default();
    // `outer` and everything below are English (the main window's layout): each part is made where
    // it is on screen (`screen`, `fill`, `text_font`), and the pointer is read in English to find
    // the column or row under it.
    let screen = winui::rect;
    let english = |q: Pos2| pos2(winui::mx(q.x), q.y);
    // Focus follows the last press: inside the list focuses it, anywhere else unfocuses it.
    if ui.input(|i| i.pointer.any_pressed()) {
        st.focused = ui
            .ctx()
            .pointer_interact_pos()
            .is_some_and(|p| screen(outer).contains(p));
    }
    let p = ui.painter_at(screen(outer));
    let (l, t, r, b) = (
        outer.left(),
        outer.top(),
        outer.right() - 1.0,
        outer.bottom() - 1.0,
    );
    let modern = row_h() > 18.0;
    if modern {
        p.rect_filled(screen(outer), 6.0, BG());
        p.rect_stroke(
            screen(outer),
            6.0,
            egui::Stroke::new(1.0, FRAME()),
            egui::StrokeKind::Inside,
        );
    } else {
        winui::solid(&p, screen(outer), FRAME());
        fill(&p, l + 1.0, t + 1.0, r - 1.0, b - 1.0, BG());
    }
    let (cl, ct, cr, cb) = (l + 2.0, t + 2.0, r - 2.0, b - 2.0);
    let font = if modern {
        winui::segoe()
    } else {
        winui::tahoma()
    };

    // Scroll bars (notes/ui-spec/scrollbars.md), decided like Win32: each one shows when the
    // content overflows the room the other one leaves.
    let (client_w, client_h) = (cr - cl + 1.0, cb - ct + 1.0);
    let rows_h = rows.len() as f32 * row_h();
    let menu_key = keyboard(ui, st, rows, focus, client_h, &mut out);
    let (mut need_v, mut need_h) = (false, false);
    for _ in 0..2 {
        let avail_w = client_w - if need_v { scrollbar::THICK } else { 0.0 };
        let (sc, ew) = layout_for_width(st, avail_w, modern);
        let cw: f32 = sc.iter().map(|c| ew[*c]).sum();
        // An empty list has nothing to scroll to (its empty state is centred in the view).
        need_h = !rows.is_empty() && cw > avail_w + 0.5;
        need_v = rows_h > client_h - header_h() - if need_h { scrollbar::THICK } else { 0.0 };
    }
    // Inclusive right edge of the header and rows, and bottom of the rows.
    let rr = cr - if need_v { scrollbar::THICK } else { 0.0 };
    let rb = cb - if need_h { scrollbar::THICK } else { 0.0 };
    let row_w = rr - cl + 1.0;
    let content_w = {
        let (sc, ew) = layout_for_width(st, row_w, modern);
        sc.iter().map(|c| ew[*c]).sum::<f32>()
    };
    st.hscroll = st.hscroll.clamp(0.0, (content_w - row_w).max(0.0)).round();

    // Column spans in display order: (slot, x, right) with the separator on the last pixel.
    let spans = |st: &ListState| {
        let (shown_cols, eff) = layout_for_width(st, row_w, modern);
        let mut v = Vec::new();
        let mut x = cl - st.hscroll;
        for c in shown_cols {
            v.push((c, x, x + eff[c] - 1.0));
            x += eff[c];
        }
        v
    };

    // ---- header interaction ----------------------------------------------------------------
    let header = Rect::from_min_max(pos2(cl, ct), pos2(rr + 1.0, ct + header_h()));
    let hresp = ui.interact(
        screen(header),
        egui::Id::unique("list-header"),
        winui::pointer_drag(),
    );
    let pointer = ui.ctx().pointer_interact_pos().map(english);
    let cols = spans(st);
    let near_sep = |x: f32| {
        cols.iter()
            .find(|(_, _, x1)| (x - (x1 + 0.5)).abs() <= GRIP)
            .map(|s| s.0)
    };
    if let Some(pp) = hresp.hover_pos().map(english)
        && near_sep(pp.x).is_some()
    {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    // Resize or move is decided where the button went down: a drag is only recognised after the
    // pointer has travelled a few pixels, by then off a border's grip (user report 2026-10-29:
    // dragging a border moved the column instead of resizing it).
    if hresp.drag_started()
        && let Some(pp) = ui
            .input(|i| i.pointer.press_origin())
            .map(english)
            .or(pointer)
    {
        st.drag = match near_sep(pp.x) {
            Some(col) => Some(HeaderDrag::Resize {
                col,
                width: layout_for_width(st, row_w, modern).1[col],
                x0: pp.x,
            }),
            None => cols
                .iter()
                .find(|(_, x0, x1)| pp.x >= *x0 && pp.x <= *x1)
                .map(|(col, _, _)| HeaderDrag::Press {
                    col: *col,
                    x0: pp.x,
                }),
        };
    }
    if let (Some(drag), Some(pp)) = (st.drag, pointer) {
        match drag {
            HeaderDrag::Resize { col, width, x0 } => {
                ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
                st.widths[col] = (width + pp.x - x0).round().max(MIN_WIDTH);
            }
            HeaderDrag::Press { col, x0 } if (pp.x - x0).abs() >= DRAG_START => {
                let start = cols.iter().find(|s| s.0 == col).map_or(x0, |s| s.1);
                st.drag = Some(HeaderDrag::Move {
                    col,
                    grab: x0 - start,
                });
            }
            _ => {}
        }
    }
    // A double-click on a border fits the column to the widest of its title and cells (as in
    // Explorer).
    if hresp.double_clicked()
        && let Some(col) = pointer.and_then(|pp| near_sep(pp.x))
    {
        st.widths[col] = fit_width(&p, &font, col, rows);
        st.drag = None;
    }
    if hresp.drag_stopped() {
        if let (Some(HeaderDrag::Move { col, .. }), Some(pp)) = (st.drag, pointer) {
            move_column(st, &cols, col, pp.x);
        }
        st.drag = None;
    }
    // A click on a header's funnel opens its filter box instead of sorting.
    let funnel_hit = |st: &ListState, pp: Pos2| {
        cols.iter()
            .find(|(c, x0, x1)| funnel_rect(st, *c, *x0, *x1, ct).contains(pp))
            .map(|(c, x0, x1)| (*c, funnel_rect(st, *c, *x0, *x1, ct).left_bottom(), *x1))
    };
    let funnel_click = hresp
        .clicked()
        .then_some(pointer)
        .flatten()
        .and_then(|pp| funnel_hit(st, pp));
    if let Some((col, at, _)) = funnel_click {
        st.filter_open = match st.filter_open {
            Some((c, _)) if c == col => None,
            _ => Some((
                col,
                pos2(at.x.min(outer.right() - FILTER_W), ct + header_h()),
            )),
        };
        st.filter_hscroll = st.hscroll;
    }
    if hresp.clicked()
        && funnel_click.is_none()
        && let Some(pp) = pointer
        && near_sep(pp.x).is_none()
        && let Some((col, ..)) = cols.iter().find(|(_, x0, x1)| pp.x >= *x0 && pp.x <= *x1)
    {
        // Ascending → descending → unsorted (order of addition).
        st.sort = match st.sort {
            Some((c, true)) if c == *col => Some((c, false)),
            Some((c, false)) if c == *col => None,
            _ => Some((*col, true)),
        };
        out.sort_changed = true;
    }
    if hresp.secondary_clicked() {
        out.header_context = ui.ctx().pointer_interact_pos();
    }

    // ---- header painting -------------------------------------------------------------------
    let cols = spans(st);
    let content_w: f32 = cols.iter().map(|(_, x0, x1)| x1 - x0 + 1.0).sum();
    let p_all = p.clone();
    let p = p.with_clip_rect(screen(header));
    fill(&p, cl, ct, rr, ct + header_h() - 1.0, HEADER_BG());
    for (c, x0, x1) in &cols {
        let hovered = hresp
            .hover_pos()
            .map(english)
            .is_some_and(|h| h.x >= *x0 && h.x <= *x1 && st.drag.is_none());
        // The column under the pointer, and pressed while the mouse button is held on it (as
        // Windows' headers); the shared tint (winui::subtle_fill).
        let state = match (hovered, hresp.is_pointer_button_down_on()) {
            (false, _) => winui::State::Normal,
            (true, false) => winui::State::Hot,
            (true, true) => winui::State::Pressed,
        };
        let tint = winui::subtle_fill(state, crate::theme::colors());
        // The sorted column: highlighted header and a chevron at its right edge (▲ ascending,
        // ▼ descending).
        if let Some((sc, asc)) = st.sort
            && sc == *c
        {
            fill(
                &p,
                *x0,
                ct,
                x1 - 1.0,
                ct + header_h() - 1.0,
                tint.unwrap_or(SORTED_BG()),
            );
            // At the cell's right edge, vertically centred in the header.
            let cx = x1 - 9.0;
            let mid = ct + (header_h() / 2.0).floor() - 2.0;
            for i in 0..4 {
                let y = mid + if asc { i as f32 } else { 3.0 - i as f32 };
                let half = i as f32;
                fill(&p, cx - half, y, cx + half, y, SORT_ARROW());
            }
        } else if let Some(t) = tint {
            fill(&p, *x0, ct, x1 - 1.0, ct + header_h() - 1.0, t);
        }
        fill(&p, *x1, ct, *x1, ct + header_h() - 1.0, HEADER_SEP());
        let mut arrow_room = if st.sort.is_some_and(|(sc, _)| sc == *c) {
            14.0
        } else {
            0.0
        };
        // The funnel (user, 2026-10-30): always shown, faint; darker under the pointer or while
        // its box is open; blue while the column is filtered. A column too narrow for a label
        // and the funnel (Q) shows it only under the pointer.
        let filtered = st.filters[*c].is_some();
        let open = st.filter_open.is_some_and(|(fc, _)| fc == *c);
        let roomy = x1 - x0 >= 44.0;
        let show_funnel = if modern {
            filtered || hovered || open
        } else {
            roomy || filtered || hovered || open
        };
        if show_funnel {
            let fr = funnel_rect(st, *c, *x0, *x1, ct);
            let color = if filtered {
                FILTER_ON()
            } else if hovered || open {
                SORT_ARROW()
            } else {
                SORT_ARROW().gamma_multiply(0.45)
            };
            if !roomy {
                p.rect_filled(screen(fr), crate::theme::metrics().part_radius, HEADER_BG());
            }
            funnel(&p, fr, color);
            if roomy {
                arrow_room += 14.0;
            }
        }
        let full = crate::i18n::tc("column", TITLES[*c]);
        let label = winui::ellipsize(&p, &font, full, x1 - x0 - 10.0 - arrow_room);
        // A cut title shows whole under the pointer; the one-letter Q says what it is.
        if hovered && st.drag.is_none() {
            let tip = if TITLES[*c] == "Q" {
                // i18n: tooltip of the narrow "Q" column's title
                Some(crate::i18n::tc("column", "Queue"))
            } else {
                (label != full).then_some(full)
            };
            if let Some(tip) = tip {
                hresp.clone().on_hover_text_at_pointer(tip);
            }
        }
        let clip = p.with_clip_rect(screen(Rect::from_x_y_ranges(
            *x0..=*x1,
            ct..=ct + header_h(),
        )));
        let hy = ct + ((header_h() + 7.0) / 2.0).floor();
        text_font(&clip, font.clone(), x0 + 6.0, hy, &label, TEXT());
    }
    if modern {
        fill(
            &p,
            cl,
            ct + header_h() - 1.0,
            rr,
            ct + header_h() - 1.0,
            HEADER_SEP(),
        );
    }

    // ---- body ------------------------------------------------------------------------------
    let p = p_all;
    let body_top = ct + header_h();
    let body = Rect::from_min_max(pos2(cl, body_top), pos2(rr + 1.0, rb + 1.0));
    out.body = Some(body);
    let resp = ui.interact(
        screen(body),
        egui::Id::unique("download-list"),
        winui::pointer_drag(),
    );
    // For screen readers: the list, its rows below, and the row with the dotted outline
    // as the one they read (the list keeps its keys itself, not egui's focus).
    winui::expose(
        &resp,
        egui::Role::ListBox,
        crate::i18n::t("Downloads"),
        true,
        None,
    );
    if let Some(f) = focus {
        let row = egui::Id::unique(("download-row", f));
        ui.ctx()
            .accesskit_node_builder(resp.id, |n| n.set_active_descendant(row.accesskit_id()));
    }
    let bp = p.with_clip_rect(screen(body));
    // Fully visible rows; the list scrolls by whole rows until the last one is fully shown.
    let page_rows = (body.height() / row_h()).floor().max(1.0);
    let max_scroll = (rows.len() as f32 - page_rows).max(0.0) * row_h();
    if resp.hovered() {
        st.scroll += winui::wheel_rows(ui, row_h(), page_rows) * row_h();
        st.hscroll -= ui.input(|i| i.smooth_scroll_delta.x);
    }
    // Rows out of view are there for screen readers too (#111), as many as `reachable_rows`
    // gives around the view; one asked to scroll into view shows from this frame on.
    let row_id = |idx: usize| egui::Id::unique(("download-row", idx));
    // The number of rows is the list's (UI Automation reads a row's "of M" from its list).
    let a11y = ui
        .ctx()
        .accesskit_node_builder(resp.id, |n| n.set_size_of_set(rows.len()))
        .is_some();
    let page = (body.height() / row_h()).floor().max(1.0) as usize;
    let reach = winui::reachable_rows(rows.len(), (st.scroll / row_h()).round() as usize, page);
    if a11y && let Some(idx) = winui::row_asked_for(ui, reach.clone(), row_id) {
        st.scroll_to = Some(idx);
    }
    if let Some(row) = st.scroll_to.take() {
        let top = row as f32 * row_h();
        if top < st.scroll {
            st.scroll = top;
        } else if top + row_h() > st.scroll + page_rows * row_h() {
            st.scroll = top + row_h() - page_rows * row_h();
        }
    }
    st.scroll = st.scroll.clamp(0.0, max_scroll);
    st.hscroll = st.hscroll.clamp(0.0, (content_w - row_w).max(0.0)).round();
    let shown_scroll = (st.scroll / row_h()).round() * row_h();
    let first = (shown_scroll / row_h()).floor() as usize;
    let offset = 0.0;
    let visible = (body.height() / row_h()).ceil() as usize + 1;
    let last_right = cols.last().map_or(cl, |s| s.2);
    let mut cut_under_pointer = None;
    let keys_in_use = winui::keyboard_in_use(ui.ctx());
    // A screen reader's row: the name, then Status and Size, selected or not, its place among all
    // the rows, where it is (out of view: above or below the list).
    let ppp = ui.ctx().pixels_per_point();
    // The rows are the list's children (UI Automation reads "12 of 40" from the list, and finds
    // them under it): made as widgets of a Ui whose parent is the list, clipped to nothing so
    // they take no pointer from it.
    let mut rows_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(screen(body))
            .accessibility_parent(resp.id),
    );
    rows_ui.set_clip_rect(Rect::NOTHING);
    let expose_row = |idx: usize, row: &Row, picked: bool| {
        let top = body_top + (idx as f32 - first as f32) * row_h() - offset;
        let bounds = screen(Rect::from_min_size(
            pos2(cl, top),
            egui::vec2(rr - cl, row_h()),
        ));
        rows_ui.interact(bounds, row_id(idx), egui::Sense::hover());
        let spoken = [0usize, 3, 2]
            .iter()
            .map(|c| row.cells[*c].as_str())
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        ui.ctx().accesskit_node_builder(row_id(idx), |n| {
            n.set_role(egui::Role::ListBoxOption);
            n.set_label(spoken);
            n.set_selected(picked);
            // AccessKit counts from 0 (UI Automation and AT-SPI say idx + 1).
            n.set_position_in_set(idx);
            n.set_size_of_set(rows.len());
            n.add_action(egui::accesskit::Action::ScrollIntoView);
            n.set_bounds(egui::accesskit::Rect {
                x0: (bounds.left() * ppp) as f64,
                y0: (bounds.top() * ppp) as f64,
                x1: (bounds.right() * ppp) as f64,
                y1: (bounds.bottom() * ppp) as f64,
            });
        });
    };
    // In the list's order: those above the view, those in it (below, as they are drawn), then
    // those under it.
    let reach = if a11y {
        winui::reachable_rows(rows.len(), first, page)
    } else {
        0..0
    };
    let expose_out_of_view = |range: std::ops::Range<usize>| {
        for idx in range {
            if let Some(row) = rows.get(idx) {
                expose_row(idx, row, selected.get(idx).copied().unwrap_or(false));
            }
        }
    };
    let hover_idx = resp.hover_pos().map(english).and_then(|pos| {
        if body.contains(pos) && pos.x <= last_right {
            let i = ((pos.y - body_top + shown_scroll) / row_h()).floor() as usize;
            (i < rows.len()).then_some(i)
        } else {
            None
        }
    });
    expose_out_of_view(reach.start..reach.end.min(first));
    for k in 0..visible {
        let idx = first + k;
        let top = body_top + k as f32 * row_h() - offset;
        if let Some(row) = rows.get(idx) {
            let picked = selected.get(idx).copied().unwrap_or(false);
            expose_row(idx, row, picked);
            let sel = picked && st.focused;
            let has_focus = focus == Some(idx) && st.focused && keys_in_use;
            if modern {
                let pill_rc = Rect::from_min_max(
                    pos2(cl + 4.0, top + 2.0),
                    pos2(last_right - 4.0, top + row_h() - 3.0),
                );
                if sel {
                    bp.rect_filled(screen(pill_rc), 6.0, SELECTED());
                    let bar = Rect::from_min_size(
                        pos2(cl + 5.0, top + ((row_h() - 20.0) / 2.0).floor()),
                        egui::vec2(3.0, 20.0),
                    );
                    bp.rect_filled(screen(bar), 1.5, winui::accent());
                } else if picked {
                    bp.rect_filled(screen(pill_rc), 6.0, winui::SELECTED_INACTIVE());
                } else if hover_idx == Some(idx) {
                    bp.rect_filled(
                        screen(pill_rc),
                        6.0,
                        crate::theme::colors().toolbar_hot_faint,
                    );
                }
                if has_focus {
                    bp.rect_stroke(
                        screen(pill_rc),
                        6.0,
                        egui::Stroke::new(1.0, crate::theme::colors().focus_ring),
                        egui::StrokeKind::Inside,
                    );
                }
            } else if sel || has_focus {
                let (right, bottom) = (last_right - 1.0, top + row_h() - 2.0);
                if sel {
                    fill(&bp, cl, top, right, bottom, SELECTED());
                }
                if has_focus {
                    let dot = if sel {
                        crate::theme::colors().selection_focus
                    } else {
                        TEXT()
                    };
                    dotted(&bp, cl, top, right, bottom, dot);
                }
            } else if picked {
                let bottom = top + row_h() - 2.0;
                fill(
                    &bp,
                    cl,
                    top,
                    last_right - 1.0,
                    bottom,
                    winui::SELECTED_INACTIVE(),
                );
            }
            let icon_y = top + ((row_h() - 16.0) / 2.0).floor().max(0.0);
            let text_y = if modern {
                top + ((row_h() + 9.0) / 2.0).floor()
            } else {
                top + ((row_h() + 7.0) / 2.0).floor()
            };
            let hover_pt = resp.hover_pos().map(english);
            for (c, x0, x1) in &cols {
                let cell = Rect::from_min_max(pos2(*x0, top), pos2(*x1 - 1.0, top + row_h()));
                let clip = bp.with_clip_rect(screen(cell.intersect(body)));
                if modern && *c == 0 {
                    let mut ix = x0 + 10.0;
                    if let Some(btn_rc) = action_rect(&cols, top, modern) {
                        let btn_hot = hover_idx == Some(idx)
                            && hover_pt.is_some_and(|hp| btn_rc.contains(hp));
                        paint_row_action(&clip, btn_rc, row, btn_hot, sel);
                        if btn_hot {
                            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                            cut_under_pointer = Some(action_tooltip(row));
                        }
                        ix = x0 + 42.0;
                    }
                    icons.paint_in(ui, &bp, row.icon.clone(), pos2(ix, icon_y));
                    let tx = ix + 22.0;
                    let value = cell_text(0, &row.cells[0]);
                    let room = (x1 - tx - 10.0).max(0.0);
                    let s = winui::ellipsize(&bp, &font, &value, room);
                    if hover_pt.is_some_and(|h| cell.contains(h))
                        && action_rect(&cols, top, modern)
                            .is_none_or(|b| hover_pt.is_none_or(|hp| !b.contains(hp)))
                        && s != value
                    {
                        cut_under_pointer = Some(row.cells[0].clone());
                    }
                    let has_bar = matches!(row.status_group, "Downloading" | "Stopped" | "Error")
                        && (row.progress.is_some() || row.status_group == "Downloading");
                    let name_y = if has_bar { top + 20.0 } else { text_y };
                    let name_ink = if sel {
                        winui::SELECTION_TEXT()
                    } else {
                        TEXT()
                    };
                    text_font(&clip, font.clone(), tx, name_y, &s, name_ink);
                    if has_bar && room > 24.0 {
                        let track = Rect::from_min_size(pos2(tx, top + 29.0), egui::vec2(room, 6.0));
                        clip.rect_filled(
                            screen(track),
                            3.0,
                            crate::theme::colors().meter_track,
                        );
                        let bar_color = match row.status_group {
                            "Downloading" => crate::theme::colors().progress_bar,
                            "Stopped" => {
                                if crate::theme::dark_surface() {
                                    Color32::from_rgb(251, 191, 36)
                                } else {
                                    Color32::from_rgb(245, 158, 11)
                                }
                            }
                            "Error" => winui::ERROR(),
                            _ => SORT_ARROW(),
                        };
                        let pct = row.progress.unwrap_or(0.18).clamp(0.0, 1.0);
                        let fw = (room * pct).round().max(if pct > 0.0 { 4.0 } else { 0.0 });
                        if fw > 0.0 {
                            let fill_rc =
                                Rect::from_min_size(pos2(tx, top + 29.0), egui::vec2(fw, 6.0));
                            clip.rect_filled(screen(fill_rc), 3.0, bar_color);
                        }
                    }
                    continue;
                }
                if modern && *c == 3 {
                    let (full_status, cut) =
                        paint_modern_status(&clip, &font, (*x0, *x1), top, row);
                    if hover_pt.is_some_and(|h| cell.contains(h)) {
                        let tip = status_tip(idx);
                        if !tip.is_empty() {
                            cut_under_pointer = Some(tip);
                        } else if cut {
                            cut_under_pointer = Some(full_status);
                        }
                    }
                    continue;
                }
                let tx = if *c == 0 {
                    icons.paint_in(ui, &bp, row.icon.clone(), pos2(x0 + 4.0, icon_y));
                    x0 + 22.0
                } else {
                    x0 + 6.0
                };
                if *c == 1 && row.queued {
                    icons.paint_in(
                        ui,
                        &bp,
                        IconKey::Silk("page_white_stack"),
                        pos2(x0 + 2.0, icon_y),
                    );
                }
                if *c == SOURCE {
                    let cut = chip(
                        &clip,
                        &font,
                        (*x0, *x1),
                        top,
                        &row.cells[*c],
                    );
                    if cut && hover_pt.is_some_and(|h| cell.contains(h)) {
                        cut_under_pointer = Some(crate::i18n::t(&row.cells[*c]).to_owned());
                    }
                    continue;
                }
                let raw_cell = if modern && *c == 2 && !row.size_detail.is_empty() {
                    &row.size_detail
                } else {
                    &row.cells[*c]
                };
                let value = cell_text(*c, raw_cell);
                let s = winui::ellipsize(&bp, &font, &value, x1 - tx - 3.0);
                if hover_pt.is_some_and(|h| cell.contains(h)) {
                    let tip = if *c == 3 {
                        status_tip(idx)
                    } else {
                        String::new()
                    };
                    if !tip.is_empty() {
                        cut_under_pointer = Some(tip);
                    } else if s != value {
                        cut_under_pointer = Some(raw_cell.clone());
                    }
                }
                let pct_opt = if *c == 3 {
                    status_percent(&row.cells[*c])
                } else {
                    None
                };
                if let Some(pct) = pct_opt
                    && x1 - x0 > 16.0
                {
                    let bar_w = (x1 - x0 - 8.0).max(0.0);
                    let track = Rect::from_min_size(
                        pos2(x0 + 4.0, top + row_h() - 4.0),
                        egui::vec2(bar_w, 2.0),
                    );
                    clip.rect_filled(
                        screen(track),
                        1.0,
                        crate::theme::colors().meter_track,
                    );
                    let fill_rect = Rect::from_min_size(
                        pos2(x0 + 4.0, top + row_h() - 4.0),
                        egui::vec2((bar_w * pct).round(), 2.0),
                    );
                    clip.rect_filled(
                        screen(fill_rect),
                        1.0,
                        crate::theme::colors().progress_bar,
                    );
                }
                let failed = row.status_group == "Error" && (*c == 3 || *c == 7);
                let live_rate = modern && *c == 5 && row.status_group == "Downloading";
                let ink = if sel {
                    winui::SELECTION_TEXT()
                } else if failed {
                    winui::ERROR()
                } else if live_rate {
                    crate::theme::colors().progress_bar
                } else {
                    TEXT()
                };
                text_font(&clip, font.clone(), tx, text_y, &s, ink);
            }
            if modern {
                let gy = top + row_h() - 1.0;
                fill(&bp, cl + 10.0, gy, last_right - 10.0, gy, GRID_H());
            }
        }
        if !modern && !rows.is_empty() {
            let gy = top + row_h() - 1.0;
            fill(&bp, cl, gy, rr, gy, GRID_H());
        }
    }
    expose_out_of_view(reach.start.max(first + visible)..reach.end);
    if let Some(full) = cut_under_pointer {
        resp.clone().on_hover_text_at_pointer(full);
    }
    filter_box(ui, st);
    if !modern {
        for (_, _, x1) in cols.iter().filter(|_| !rows.is_empty()) {
            fill(&bp, x1 + 1.0, body_top, x1 + 1.0, rb, GRID_V_RIGHT());
        }
    }
    // While a column is dragged: a translucent copy of its header follows the pointer, and a
    // 2 px mark shows where it will land.
    if let (Some(HeaderDrag::Move { col, grab }), Some(pp)) = (st.drag, pointer) {
        let w = layout_for_width(st, row_w, modern).1[col];
        let ghost = Rect::from_min_size(pos2(pp.x - grab, ct), egui::vec2(w, header_h()));
        let top = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::unique("col-ghost"),
        ));
        top.rect_filled(screen(ghost), 0.0, crate::theme::colors().drag_ghost);
        text_font(
            &top,
            font.clone(),
            ghost.left() + 6.0,
            ct + 15.0,
            crate::i18n::tc("column", TITLES[col]),
            TEXT(),
        );
        let mark = drop_x(&cols, pp.x);
        fill(
            &top,
            mark - 1.0,
            ct,
            mark,
            ct + header_h() - 1.0,
            winui::accent(),
        );
    }

    // ---- scroll bars -----------------------------------------------------------------------
    // Vertical: beside the header too (client top to the corner), in rows.
    if need_v {
        // At the trailing edge: the left in RTL.
        let track = screen(Rect::from_min_max(
            pos2(rr + 1.0, ct),
            pos2(cr + 1.0, rb + 1.0),
        ));
        let mut pos_rows = shown_scroll / row_h();
        if scrollbar::bar(
            ui,
            egui::Id::unique("list-vbar"),
            track,
            true,
            rows.len() as f32,
            page_rows,
            1.0,
            &mut pos_rows,
        ) {
            st.scroll = pos_rows.round() * row_h();
            ui.ctx().request_repaint();
        }
    }
    // Horizontal: under the rows, in pixels.
    if need_h {
        let track = screen(Rect::from_min_max(
            pos2(cl, rb + 1.0),
            pos2(rr + 1.0, cb + 1.0),
        ));
        let mut x = st.hscroll;
        if scrollbar::bar(
            ui,
            egui::Id::unique("list-hbar"),
            track,
            false,
            content_w,
            row_w,
            15.0,
            &mut x,
        ) {
            st.hscroll = x.round();
            ui.ctx().request_repaint();
        }
    }
    if need_v && need_h {
        scrollbar::corner(ui, rr + 1.0, rb + 1.0);
    }

    // ---- row hit testing -------------------------------------------------------------------
    let row_at = |pos: Pos2| -> Option<usize> {
        if !body.contains(pos) || pos.x > last_right {
            return None;
        }
        let idx = ((pos.y - body_top + shown_scroll) / row_h()).floor() as usize;
        (idx < rows.len()).then_some(idx)
    };
    let mods = ui.input(|i| i.modifiers);
    if let Some(on_screen) = resp.interact_pointer_pos()
        && let pos = english(on_screen)
        && body.contains(pos)
    {
        let on_action_btn = row_at(pos).and_then(|i| {
            let row_top = body_top + (i as f32 - first as f32) * row_h() - offset;
            let hit = action_rect(&cols, row_top, modern).is_some_and(|rc| rc.contains(pos));
            hit.then_some(i)
        });
        if resp.clicked() {
            if let Some(i) = on_action_btn {
                out.row_action = Some(i);
                out.pick = Some(Pick::Only(i));
            } else {
                // Windows: Ctrl+click puts a row in or out, Shift+click selects from the anchor,
                // Ctrl+Shift+click adds that range; a click on blank space clears (not with Ctrl).
                out.pick = match (row_at(pos), mods.command, mods.shift) {
                    (Some(i), false, false) => Some(Pick::Only(i)),
                    (Some(i), true, false) => Some(Pick::Toggle(i)),
                    (Some(to), ctrl, true) => Some(Pick::Range { to, add: ctrl }),
                    (None, false, _) => Some(Pick::Clear),
                    (None, true, _) => None,
                };
            }
        }
        if resp.double_clicked() && on_action_btn.is_none() {
            out.double_clicked = row_at(pos);
        }
        if resp.secondary_clicked() {
            // A right-click inside the selection keeps it (the menu acts on all of it); outside,
            // the row alone is selected; on blank space the selection is cleared. The menu opens
            // at the pointer (IDM opens the blank-space one at the list's centre; user decision).
            let row = row_at(pos);
            out.pick = Some(match row {
                Some(i) if selected.get(i).copied().unwrap_or(false) => Pick::Focus(i),
                Some(i) => Pick::Only(i),
                None => Pick::Clear,
            });
            out.context = Some((row, on_screen));
        }
    }
    // Shift+F10 / the Menu key: the focused row's menu, under it.
    if menu_key {
        let at = focus.map_or(body.left_top() + egui::vec2(20.0, 4.0), |f| {
            pos2(
                cl + 20.0,
                (body_top + (f as f32 + 1.0) * row_h() - shown_scroll)
                    .clamp(body.top(), body.bottom()),
            )
        });
        // `at` is English; the menu opens at its screen point.
        out.context = Some((focus, pos2(winui::mx(at.x), at.y)));
    }

    // A selection box, dragged from blank space (below the rows or right of the columns), as in
    // Explorer: the rows it touches are selected, Ctrl adds them; dragging past the top or bottom
    // scrolls.
    if resp.drag_started()
        && let Some(origin) = ui.input(|i| i.pointer.press_origin()).map(english)
        && body.contains(origin)
        && row_at(origin).is_none()
    {
        st.marquee = Some((
            pos2(origin.x + st.hscroll, origin.y - body_top + shown_scroll),
            mods.command,
        ));
    }
    if let Some((start, add)) = st.marquee {
        if let Some(pp) = pointer {
            let dt = ui.input(|i| i.stable_dt).min(0.1);
            let step = 30.0 * row_h() * dt;
            if pp.y < body.top() {
                st.scroll = (st.scroll - step).max(0.0);
                ui.ctx().request_repaint();
            } else if pp.y > body.bottom() {
                st.scroll = (st.scroll + step).min(max_scroll);
                ui.ctx().request_repaint();
            }
            let now = pos2(pp.x + st.hscroll, pp.y - body_top + st.scroll);
            let (y0, y1) = (start.y.min(now.y), start.y.max(now.y));
            let (x0, x1) = (start.x.min(now.x), start.x.max(now.x));
            let touches_columns = x0 <= last_right + st.hscroll && x1 >= cl + st.hscroll;
            let picked: Vec<usize> = if touches_columns {
                let first = (y0.max(0.0) / row_h()).floor() as usize;
                let last = (y1.max(0.0) / row_h()).floor() as usize;
                (first..=last.min(rows.len().saturating_sub(1)))
                    .filter(|i| *i < rows.len())
                    .collect()
            } else {
                Vec::new()
            };
            out.pick = Some(Pick::Rows { rows: picked, add });
            let screen = Rect::from_min_max(
                pos2(x0 - st.hscroll, y0 + body_top - st.scroll),
                pos2(x1 - st.hscroll, y1 + body_top - st.scroll),
            );
            let bp = p.with_clip_rect(winui::rect(body));
            let screen = winui::rect(screen);
            bp.rect_filled(screen, 0.0, crate::theme::colors().marquee);
            bp.rect_stroke(
                screen,
                0.0,
                egui::Stroke::new(1.0, crate::theme::colors().marquee_border),
                egui::StrokeKind::Inside,
            );
        }
        if resp.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
            st.marquee = None;
            out.pick = Some(Pick::BoxDone);
        }
    }
    out
}

/// A cell's text as drawn in column `col`: data keeps its own direction in the interface's — a
/// file name or a description as a first-strong isolate (an Arabic name reads right to left in
/// an English window, a Latin one left to right in an Arabic window), a date left to right. The
/// cut happens inside the isolate, so the "..." ends the name.
/// The width that shows column `col`'s title and every cell whole (its icon, its padding).
fn fit_width(p: &egui::Painter, font: &egui::FontId, col: usize, rows: &[Row]) -> f32 {
    let measure = |s: &str| winui::text_width_font(p, font.clone(), s);
    // The title, with room for the sort arrow and the filter funnel.
    let title = measure(crate::i18n::tc("column", TITLES[col])) + 6.0 + 24.0;
    let lead = if col == 0 { 22.0 } else { 6.0 };
    let cells = rows
        .iter()
        .map(|r| lead + measure(&cell_text(col, crate::i18n::t(&r.cells[col]))) + 8.0)
        .fold(0.0, f32::max);
    match col {
        // The queue icon and the source chip have their own sizes.
        1 | SOURCE => DEFAULT_WIDTHS[col],
        _ => title.max(cells).ceil().max(MIN_WIDTH),
    }
}

fn cell_text(col: usize, s: &str) -> String {
    match col {
        0 | 7 => crate::i18n::isolate(s),
        // Dates, a peer count and a ratio read left to right.
        // Dates, a rate, a peer count and a ratio read left to right.
        6 | ADDED | UPLOAD | PEERS | RATIO => crate::i18n::ltr(s),
        _ => s.to_owned(),
    }
}

/// A focus outline: every other pixel of the rectangle's edge (inclusive pixel bounds).
fn dotted(p: &egui::Painter, l: f32, t: f32, r: f32, b: f32, color: Color32) {
    let mut x = l;
    while x <= r {
        fill(p, x, t, x, t, color);
        fill(p, x, b, x, b, color);
        x += 2.0;
    }
    let mut y = t;
    while y <= b {
        fill(p, l, y, l, y, color);
        fill(p, r, y, r, y, color);
        y += 2.0;
    }
}

/// A Source chip: a rounded pill, green "Web" or purple "Torrent", white text, at the cell's
/// left of the cell `x0..x1`. A label too long for the cell (a translation in a narrow column)
/// is cut with "..."; returns whether it was.
fn chip(
    p: &egui::Painter,
    font: &egui::FontId,
    (x0, x1): (f32, f32),
    top: f32,
    label: &str,
) -> bool {
    if label.is_empty() {
        return false;
    }
    let fill = if label == "Torrent" {
        crate::theme::colors().chip_torrent
    } else {
        crate::theme::colors().chip_web
    };
    // The cell holds the English key (filters match on it); the chip shows it translated.
    let full = crate::i18n::t(label);
    // Room: the pill starts 5 px in, pads its text 6 px each side, and ends 3 px before the cell.
    let label = &winui::ellipsize(p, font, full, (x1 - x0 - 5.0 - 12.0 - 3.0).max(0.0));
    let w = winui::text_width_font(p, font.clone(), label).round();
    let modern = row_h() > 18.0;
    let pill_h = if modern {
        20.0
    } else {
        (row_h() - 4.0).min(15.0)
    };
    let pill_top = top + ((row_h() - pill_h) / 2.0).floor();
    let pill = Rect::from_min_size(pos2(x0 + 5.0, pill_top), egui::vec2(w + 12.0, pill_h));
    let bg = if modern {
        fill.gamma_multiply(0.15)
    } else {
        fill
    };
    let ink = if modern {
        fill
    } else {
        crate::theme::colors().chip_text
    };
    p.rect_filled(winui::rect(pill), pill_h / 2.0, bg);
    text_font(
        p,
        font.clone(),
        pill.left() + 6.0,
        top + ((row_h() + if modern { 8.0 } else { 7.0 }) / 2.0).floor(),
        label,
        ink,
    );
    label != full
}

/// English bounds of the inline quick-action button at the start of File Name in modern mode.
fn action_rect(cols: &[(usize, f32, f32)], top: f32, modern: bool) -> Option<Rect> {
    if !modern {
        return None;
    }
    let (_, x0, x1) = cols.iter().find(|(c, _, _)| *c == 0)?;
    if *x1 - *x0 < 92.0 {
        return None;
    }
    let cy = top + (row_h() / 2.0).floor();
    Some(Rect::from_center_size(pos2(*x0 + 21.0, cy), egui::vec2(26.0, 26.0)))
}

fn action_tooltip(row: &Row) -> String {
    let key = if row.status_group == "Downloading" {
        "Pause"
    } else if row.can_resume {
        "Resume"
    } else {
        "Open folder"
    };
    crate::i18n::t(key).to_owned()
}

/// Paints the circular FDM-style inline Pause / Resume / Open Folder button inside `rc` (English).
fn paint_row_action(p: &egui::Painter, rc: Rect, row: &Row, hot: bool, sel: bool) {
    let sc = winui::rect(rc);
    let (cx, cy) = (sc.center().x, sc.center().y);
    let dark = crate::theme::dark_surface();
    if row.status_group == "Downloading" {
        let fg = if sel && !dark {
            winui::accent()
        } else {
            crate::theme::colors().progress_bar
        };
        let bg = fg.gamma_multiply(if hot { 0.24 } else { 0.12 });
        p.rect_filled(sc, 13.0, bg);
        let b1 = Rect::from_center_size(pos2(cx - 3.0, cy), egui::vec2(2.6, 10.0));
        let b2 = Rect::from_center_size(pos2(cx + 3.0, cy), egui::vec2(2.6, 10.0));
        p.rect_filled(b1, 1.0, fg);
        p.rect_filled(b2, 1.0, fg);
    } else if row.can_resume {
        let fg = match row.status_group {
            "Stopped" => {
                if dark {
                    Color32::from_rgb(251, 191, 36)
                } else {
                    Color32::from_rgb(217, 119, 6)
                }
            }
            "Error" => winui::ERROR(),
            _ => crate::theme::colors().progress_bar,
        };
        let bg = fg.gamma_multiply(if hot { 0.24 } else { 0.12 });
        p.rect_filled(sc, 13.0, bg);
        p.add(egui::Shape::convex_polygon(
            vec![
                pos2(cx - 3.0, cy - 5.0),
                pos2(cx + 5.5, cy),
                pos2(cx - 3.0, cy + 5.0),
            ],
            fg,
            egui::Stroke::NONE,
        ));
    } else {
        let fg = if hot {
            crate::theme::colors().progress_bar
        } else if dark {
            Color32::from_rgb(148, 163, 184)
        } else {
            Color32::from_rgb(71, 85, 105)
        };
        let bg = if hot {
            crate::theme::colors().progress_bar.gamma_multiply(0.16)
        } else {
            crate::theme::colors().toolbar_hot
        };
        p.rect_filled(sc, 13.0, bg);
        let tab = Rect::from_min_size(pos2(cx - 6.0, cy - 4.5), egui::vec2(5.0, 2.5));
        let body = Rect::from_min_size(pos2(cx - 6.0, cy - 2.5), egui::vec2(12.0, 7.5));
        p.rect_filled(tab, 1.0, fg);
        p.rect_filled(body, 1.5, fg);
    }
}

/// Paints a color-coded status badge in modern mode ("Downloading 11.64%" vs "Paused 11.64%" vs "Complete").
fn paint_modern_status(
    p: &egui::Painter,
    font: &egui::FontId,
    (x0, x1): (f32, f32),
    top: f32,
    row: &Row,
) -> (String, bool) {
    let pct_str = status_percent(&row.cells[3]).map(|_| row.cells[3].as_str());
    let dl_key = "Downloading";
    let paused_key = "Paused";
    let wide = x1 - x0 >= 185.0;
    let full = match row.status_group {
        "Downloading" => match pct_str {
            Some(pct) if wide => format!("{}  {}", crate::i18n::t(dl_key), pct),
            Some(pct) => pct.to_owned(),
            None if row.cells[3].is_empty() => crate::i18n::t(dl_key).to_owned(),
            None => row.cells[3].clone(),
        },
        "Stopped" => match pct_str {
            Some(pct) if wide => format!("{}  {}", crate::i18n::t(paused_key), pct),
            Some(pct) => pct.to_owned(),
            None => crate::i18n::t(paused_key).to_owned(),
        },
        _ => row.cells[3].clone(),
    };
    if full.is_empty() {
        return (full, false);
    }
    let dark = crate::theme::dark_surface();
    let (fg, bg) = match row.status_group {
        "Downloading" => {
            let fg = if dark {
                Color32::from_rgb(96, 165, 250)
            } else {
                Color32::from_rgb(29, 78, 216)
            };
            (fg, crate::theme::colors().progress_bar.gamma_multiply(0.13))
        }
        "Stopped" => {
            let fg = if dark {
                Color32::from_rgb(251, 191, 36)
            } else {
                Color32::from_rgb(180, 83, 9)
            };
            (fg, fg.gamma_multiply(0.15))
        }
        "Complete" => {
            let fg = if dark {
                Color32::from_rgb(74, 222, 128)
            } else {
                Color32::from_rgb(21, 128, 61)
            };
            (fg, crate::theme::colors().meter_fill.gamma_multiply(0.14))
        }
        "Error" => {
            let fg = winui::ERROR();
            (fg, fg.gamma_multiply(0.14))
        }
        _ => {
            let fg = SORT_ARROW();
            (fg, fg.gamma_multiply(0.14))
        }
    };
    let max_text = (x1 - x0 - 34.0).max(0.0);
    let shown = winui::ellipsize(p, font, &full, max_text);
    let tw = winui::text_width_font(p, font.clone(), &shown).round();
    let pill_h = 22.0;
    let pill_top = top + ((row_h() - pill_h) / 2.0).floor();
    let pill = Rect::from_min_size(pos2(x0 + 6.0, pill_top), egui::vec2(tw + 26.0, pill_h));
    let pill_s = winui::rect(pill);
    p.rect_filled(pill_s, 11.0, bg);

    let mark_box = winui::rect(Rect::from_center_size(
        pos2(x0 + 15.0, pill_top + 11.0),
        egui::vec2(8.0, 8.0),
    ));
    let mc = mark_box.center();
    match row.status_group {
        "Complete" => {
            let st = egui::Stroke::new(1.6, fg);
            p.line_segment([pos2(mc.x - 3.5, mc.y + 0.2), pos2(mc.x - 1.0, mc.y + 2.8)], st);
            p.line_segment([pos2(mc.x - 1.0, mc.y + 2.8), pos2(mc.x + 3.8, mc.y - 2.6)], st);
        }
        "Stopped" => {
            let b1 = Rect::from_center_size(pos2(mc.x - 1.8, mc.y), egui::vec2(1.8, 7.0));
            let b2 = Rect::from_center_size(pos2(mc.x + 1.8, mc.y), egui::vec2(1.8, 7.0));
            p.rect_filled(b1, 0.8, fg);
            p.rect_filled(b2, 0.8, fg);
        }
        _ => {
            p.circle_filled(mc, 3.0, fg);
        }
    }

    text_font(
        p,
        font.clone(),
        x0 + 24.0,
        top + ((row_h() + 8.0) / 2.0).floor(),
        &shown,
        fg,
    );
    let cut = shown != full;
    (full, cut)
}

fn status_percent(s: &str) -> Option<f32> {
    if !s.contains('%') {
        return None;
    }
    let num: String = s
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
        .map(|c| if c == ',' { '.' } else { c })
        .collect();
    num.parse::<f32>().ok().map(|v| (v / 100.0).clamp(0.0, 1.0))
}

/// Where a column's funnel sits in its header: at the right, left of the sort chevron.
fn funnel_rect(st: &ListState, col: usize, x0: f32, x1: f32, ct: f32) -> Rect {
    let sorted = st.sort.is_some_and(|(sc, _)| sc == col);
    let right = x1 - if sorted { 18.0 } else { 4.0 };
    let left = (right - 12.0).max(x0 + 2.0);
    let fy = ct + ((header_h() - 12.0) / 2.0).floor();
    Rect::from_min_max(pos2(left, fy), pos2(right, fy + 12.0))
}

/// A small funnel: a triangle over a stem, in English rect `rc`.
fn funnel(p: &egui::Painter, rc: Rect, color: Color32) {
    let rc = winui::rect(rc);
    let (cx, top) = (rc.center().x, rc.top() + 1.0);
    p.add(egui::Shape::convex_polygon(
        vec![
            pos2(cx - 5.0, top),
            pos2(cx + 5.0, top),
            pos2(cx + 1.0, top + 5.0),
            pos2(cx - 1.0, top + 5.0),
        ],
        color,
        egui::Stroke::NONE,
    ));
    winui::fill(p, cx - 1.0, top + 5.0, cx, top + 10.0, color);
}

const FILTER_W: f32 = 230.0;

/// The open filter box under a header: a checkbox per value, or a "contains" search box; it
/// filters as you change it. Escape or a click elsewhere closes it.
fn filter_box(ui: &mut Ui, st: &mut ListState) {
    let Some((col, at)) = st.filter_open else {
        return;
    };
    let values = filter_values(col);
    let h = match values {
        Some(v) => 58.0 + 18.0 * v.len() as f32,
        None => 92.0,
    };
    // Laid out in English under its header (`at`); shown where that is on screen.
    let rect = Rect::from_min_size(at, egui::vec2(FILTER_W, h));
    let shown = winui::rect(rect);
    let r = winui::rect;
    let mut close = ui.input(|i| i.key_pressed(egui::Key::Escape));
    // A press anywhere else closes it (the toolbar, the menu bar, another header too), except
    // on its own funnel, whose click toggles it; so does the list scrolling sideways (it
    // stayed where the column was).
    let on_funnel =
        |pp: Pos2| pp.y <= at.y && pp.y >= at.y - header_h() && (pp.x - at.x).abs() <= 24.0;
    if ui.input(|i| i.pointer.any_pressed())
        && ui
            .ctx()
            .pointer_interact_pos()
            .is_some_and(|pp| !shown.contains(pp) && !on_funnel(pp))
    {
        close = true;
    }
    if st.hscroll != st.filter_hscroll {
        close = true;
    }
    egui::Area::new(egui::Id::unique(("list-filter", col)))
        .order(egui::Order::Foreground)
        .fixed_pos(shown.min)
        .show(ui.ctx(), |ui| {
            let (_, _) = ui.allocate_exact_size(rect.size(), Sense::hover());
            let p = ui.painter().clone();
            crate::chrome::popup_surface(&p, shown);
            let o = rect.min;
            match values {
                Some(vals) => {
                    // i18n: verb: label before the filter's list, i.e. show the rows that…
                    winui::text(
                        &p,
                        o.x + 10.0,
                        o.y + 20.0,
                        crate::i18n::t("Show"),
                        winui::TEXT(),
                    );
                    let mut keep: Vec<&'static str> = match &st.filters[col] {
                        Some(Filter::Values(k)) => k.clone(),
                        _ => vals.to_vec(),
                    };
                    for (i, v) in vals.iter().enumerate() {
                        let mut on = keep.contains(v);
                        let y = o.y + 28.0 + 18.0 * i as f32;
                        if winui::checkbox(ui, pos2(o.x + 12.0, y), &mut on, v, true).clicked() {
                            keep.retain(|k| k != v);
                            if on {
                                keep.push(v);
                            }
                        }
                    }
                    st.filters[col] = (keep.len() < vals.len()).then(|| {
                        Filter::Values(vals.iter().copied().filter(|v| keep.contains(v)).collect())
                    });
                    // As wide as its label in this language (#192), short of Close.
                    let w = winui::button_fit_width(
                        ui.painter(),
                        "Show all",
                        90.0,
                        FILTER_W - 90.0 - 8.0 - 10.0,
                    );
                    if winui::button(
                        ui,
                        r(Rect::from_min_size(
                            pos2(o.x + 10.0, o.y + h - 28.0),
                            egui::vec2(w, 21.0),
                        )),
                        "Show all",
                        winui::ButtonKind::Normal,
                        false,
                        None,
                    )
                    .clicked()
                    {
                        st.filters[col] = None;
                    }
                }
                None => {
                    let label = crate::i18n::fmt(
                        crate::i18n::t("{column} contains"),
                        &[("column", crate::i18n::tc("column", TITLES[col]))],
                    );
                    winui::text(&p, o.x + 10.0, o.y + 20.0, &label, winui::TEXT());
                    let mut t = match &st.filters[col] {
                        Some(Filter::Text(t)) => t.clone(),
                        _ => String::new(),
                    };
                    let edit = winui::edit(
                        ui,
                        r(Rect::from_min_size(
                            pos2(o.x + 10.0, o.y + 28.0),
                            egui::vec2(FILTER_W - 20.0, 21.0),
                        )),
                        &mut t,
                        true,
                        false,
                    );
                    if !edit.has_focus() && !ui.ctx().memory(|m| m.focused().is_some()) {
                        edit.request_focus();
                    }
                    st.filters[col] = (!t.is_empty()).then_some(Filter::Text(t));
                    if winui::button(
                        ui,
                        r(Rect::from_min_size(
                            pos2(o.x + 10.0, o.y + h - 30.0),
                            egui::vec2(90.0, 21.0),
                        )),
                        // i18n: verb: clears this filter
                        "Clear",
                        winui::ButtonKind::Normal,
                        false,
                        None,
                    )
                    .clicked()
                    {
                        st.filters[col] = None;
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        close = true;
                    }
                }
            }
            if winui::button(
                ui,
                r(Rect::from_min_size(
                    pos2(
                        o.x + FILTER_W - 90.0,
                        o.y + h - if values.is_some() { 28.0 } else { 30.0 },
                    ),
                    egui::vec2(80.0, 21.0),
                )),
                // i18n: verb: closes the filter box
                "Close",
                winui::ButtonKind::Default,
                false,
                None,
            )
            .clicked()
            {
                close = true;
            }
        });
    if close {
        st.filter_open = None;
    }
}

/// The x of the column boundary nearest to `x` (where a dragged column would be inserted).
fn drop_x(cols: &[(usize, f32, f32)], x: f32) -> f32 {
    let mut best = cols.first().map_or(x, |c| c.1);
    for (_, x0, x1) in cols {
        for edge in [*x0, x1 + 1.0] {
            if (edge - x).abs() < (best - x).abs() {
                best = edge;
            }
        }
    }
    best
}

/// Move column `col` to the boundary nearest `x` (within the visible columns' order).
fn move_column(st: &mut ListState, cols: &[(usize, f32, f32)], col: usize, x: f32) {
    let edge = drop_x(cols, x);
    // Insertion index among visible columns: number of columns whose start is left of the edge.
    let mut target = cols.iter().filter(|(_, x0, _)| *x0 < edge).count();
    let from = cols.iter().position(|c| c.0 == col).unwrap_or(0);
    if target > from {
        target -= 1;
    }
    let mut shown: Vec<usize> = cols.iter().map(|c| c.0).collect();
    let moved = shown.remove(from);
    shown.insert(target.min(shown.len()), moved);
    // Hidden or responsively collapsed columns keep their place after the shown ones.
    let hidden: Vec<usize> = st
        .order
        .iter()
        .copied()
        .filter(|c| !cols.iter().any(|s| s.0 == *c))
        .collect();
    st.order = shown.into_iter().chain(hidden).collect();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modern_responsive_layout_fits_compact_and_wide_windows() {
        let mut st = ListState::default();
        st.apply(Layout {
            order: vec![SOURCE, 0, 2, 3, 5, 4, ADDED, 1, UPLOAD, PEERS, RATIO, 6, 7],
            widths: vec![
                340.0, 24.0, 95.0, 145.0, 100.0, 115.0, 90.0, 200.0, 155.0, 68.0, 64.0, 92.0, 100.0,
            ],
            visible: vec![
                true, false, true, true, true, true, false, false, true, true, false, false, false,
            ],
            sort: None,
        });

        // Compact floating window (~716 px wide -> row_w = 708 px):
        // collapses Source & Added, expands Size to >= 132 px, and fits row_w with zero overflow.
        let (shown_compact, w_compact) = layout_for_width(&st, 708.0, true);
        assert_eq!(shown_compact, vec![0, 2, 3, 5, 4]);
        assert!(w_compact[2] >= 132.0);
        assert!(w_compact[3] >= 104.0);
        assert!(w_compact[5] >= 106.0);
        assert!(w_compact[4] >= 106.0);
        assert!(w_compact[0] >= 240.0);
        let sum_compact: f32 = shown_compact.iter().map(|&c| w_compact[c]).sum();
        assert!((sum_compact - 708.0).abs() <= 0.5);

        // Maximized window (1360 px): restores Source & Added and fills exact width.
        let (shown_wide, w_wide) = layout_for_width(&st, 1360.0, true);
        assert_eq!(shown_wide, vec![SOURCE, 0, 2, 3, 5, 4, ADDED]);
        let sum_wide: f32 = shown_wide.iter().map(|&c| w_wide[c]).sum();
        assert!((sum_wide - 1360.0).abs() <= 0.5);
    }

    #[test]
    fn data_cells_keep_their_own_direction() {
        const FSI: char = '\u{2068}';
        const LRI: char = '\u{2066}';
        const PDI: char = '\u{2069}';
        // A file name and a description: first-strong isolates, whatever their script.
        assert_eq!(cell_text(0, "تقرير.pdf"), format!("{FSI}تقرير.pdf{PDI}"));
        assert_eq!(cell_text(0, "report.zip"), format!("{FSI}report.zip{PDI}"));
        assert_eq!(cell_text(7, "ملاحظة"), format!("{FSI}ملاحظة{PDI}"));
        // Dates read left to right.
        assert_eq!(
            cell_text(ADDED, "Oct 29 2026 20:00:00"),
            format!("{LRI}Oct 29 2026 20:00:00{PDI}")
        );
        assert_eq!(cell_text(6, "Oct 29"), format!("{LRI}Oct 29{PDI}"));
        // Interface text (a status) is laid out in the interface's direction as it is.
        assert_eq!(cell_text(3, "Complete"), "Complete");
        // A cut keeps the "..." inside the isolate: the closing mark goes, the dots follow the
        // name, not the paragraph.
        let cut: String = cell_text(0, "abcdef").chars().take(4).collect::<String>() + "...";
        assert_eq!(cut, format!("{FSI}abc..."));
    }

    fn spans(st: &ListState) -> Vec<(usize, f32, f32)> {
        let mut x = 0.0;
        st.shown()
            .into_iter()
            .map(|c| {
                let s = (c, x, x + st.widths[c] - 1.0);
                x += st.widths[c];
                s
            })
            .collect()
    }

    #[test]
    fn moving_a_column_right_and_left() {
        let mut st = ListState::default();
        let cols = spans(&st);
        // Drop "File Name" (slot 0) at the boundary after "Size" (x = 62 + 150 + 20 + 75 = 307).
        move_column(&mut st, &cols, 0, 308.0);
        assert_eq!(st.order[..5], [SOURCE, 1, 2, 0, 3]);
        let cols = spans(&st);
        // And to the very left.
        move_column(&mut st, &cols, 0, 1.0);
        assert_eq!(st.order[..3], [0, SOURCE, 1]);
    }

    fn row(source: &str, status: &'static str, name: &str, queued: bool) -> Row {
        let mut cells: [String; COLS] = Default::default();
        cells[0] = name.into();
        cells[SOURCE] = source.into();
        Row {
            status_group: status,
            icon: IconKey::Silk("page_white"),
            queued,
            progress: None,
            size_detail: String::new(),
            can_resume: false,
            cells,
        }
    }

    #[test]
    fn filters_by_values_and_text() {
        let mut st = ListState::default();
        let rows = [
            row("Web", "Complete", "Setup.EXE", false),
            row("Torrent", "Downloading", "debian.iso", true),
            row("Web", "Error", "notes.pdf", false),
        ];
        let shown = |st: &ListState| rows.iter().filter(|r| matches(st, r)).count();
        assert_eq!(shown(&st), 3);
        st.filters[SOURCE] = Some(Filter::Values(vec!["Torrent"]));
        assert_eq!(shown(&st), 1);
        st.filters[SOURCE] = None;
        st.filters[3] = Some(Filter::Values(vec!["Complete", "Error"]));
        assert_eq!(shown(&st), 2);
        st.filters[0] = Some(Filter::Text("setup".into()));
        assert_eq!(shown(&st), 1, "case-insensitive, combined with the others");
        st.filters = vec![None; COLS];
        st.filters[1] = Some(Filter::Values(vec!["In a queue"]));
        assert_eq!(shown(&st), 1);
    }

    /// The slots are fixed, so a title inserted in the middle of [`TITLES`] would name the wrong
    /// column — the screenshots caught exactly that.
    #[test]
    fn each_titles_slot_names_its_own_column() {
        assert_eq!(TITLES[0], "File Name");
        assert_eq!(TITLES[6], "Last Try Date");
        assert_eq!(TITLES[7], "Description");
        assert_eq!(TITLES[ADDED], "Added");
        assert_eq!(TITLES[SOURCE], "Source");
        assert_eq!(TITLES[UPLOAD], "Upload speed");
        assert_eq!(TITLES[PEERS], "Peers");
        assert_eq!(TITLES[RATIO], "Share ratio");
    }

    #[test]
    fn hidden_columns_keep_their_place() {
        let mut st = ListState::default();
        st.visible[1] = false; // hide "Q"
        let cols = spans(&st);
        move_column(&mut st, &cols, 7, 0.0); // Description to the front
        assert_eq!(st.order[0], 7);
        assert!(st.order.contains(&1));
        assert_eq!(st.order.len(), COLS);
    }

    #[test]
    fn a_layout_saved_before_the_added_columns_still_loads() {
        let old = Layout {
            order: vec![0, 2, 1, 3, 4, 5, 6, 7],
            widths: vec![200.0, 20.0, 75.0, 65.0, 75.0, 95.0, 65.0, 300.0],
            visible: vec![true; 8],
            sort: Some((2, false)),
        };
        let mut st = ListState::default();
        st.apply(old);
        // Source first, the three torrent columns after Transfer rate, Added after Last Try Date.
        assert_eq!(
            st.order,
            [SOURCE, 0, 2, 1, 3, 4, 5, UPLOAD, PEERS, RATIO, 6, ADDED, 7]
        );
        assert_eq!(st.widths[0], 200.0, "the saved widths stay");
        assert_eq!(st.widths[ADDED], DEFAULT_WIDTHS[ADDED]);
        assert!(st.visible[ADDED]);
        assert!(st.visible[SOURCE]);
        // A layout saved before they existed gets their defaults, shown.
        assert_eq!(st.widths[UPLOAD], DEFAULT_WIDTHS[UPLOAD]);
        assert_eq!(st.widths[PEERS], DEFAULT_WIDTHS[PEERS]);
        assert_eq!(st.widths[RATIO], DEFAULT_WIDTHS[RATIO]);
        assert!(st.visible[UPLOAD] && st.visible[PEERS] && st.visible[RATIO]);
        assert_eq!(st.sort, Some((2, false)));
    }

    /// #357: a layout saved between "Source" and the torrent columns gets them in the same
    /// place, and a layout that names a column twice is still ignored.
    #[test]
    fn a_layout_saved_before_the_torrent_columns_gets_them() {
        let old = Layout {
            order: vec![SOURCE, 0, 1, 2, 3, 4, 5, 6, ADDED, 7],
            widths: vec![
                62.0, 150.0, 20.0, 75.0, 65.0, 75.0, 95.0, 65.0, 125.0, 300.0,
            ],
            visible: vec![true; 10],
            sort: None,
        };
        let mut st = ListState::default();
        st.apply(old);
        assert_eq!(
            st.order,
            [SOURCE, 0, 1, 2, 3, 4, 5, UPLOAD, PEERS, RATIO, 6, ADDED, 7]
        );
        assert!(st.widths[UPLOAD] > 0.0 && st.widths[PEERS] > 0.0 && st.widths[RATIO] > 0.0);
        // None of the torrent columns were in that layout, so all three get their defaults.
        assert_eq!(st.widths[UPLOAD], DEFAULT_WIDTHS[UPLOAD]);
        // A layout that names one column twice (and so misses another) is ignored whole.
        let bad = Layout {
            order: vec![SOURCE, 0, 1, 2, 3, 4, 5, 6, ADDED, 7, PEERS, PEERS],
            widths: vec![62.0; 12],
            visible: vec![true; 12],
            sort: None,
        };
        let mut st = ListState::default();
        st.apply(bad);
        assert_eq!(st.order, DEFAULT_ORDER.to_vec(), "the default stands");
    }

    /// A layout saved by 0.5.2 (12 slots: Peers at 10, Share ratio at 11, sorted by Peers) must
    /// keep meaning what it meant: Peers is still Peers, with its own width and its sort. Upload
    /// speed takes the new, free slot and gets its default.
    #[test]
    fn a_layout_saved_by_052_keeps_peers_as_peers() {
        let old = Layout {
            order: vec![SOURCE, 0, 1, 2, 3, 4, 5, PEERS, RATIO, 6, ADDED, 7],
            widths: vec![
                150.0, 20.0, 75.0, 65.0, 75.0, 95.0, 65.0, 300.0, 145.0, 62.0, 64.0, 92.0,
            ],
            visible: vec![true; 12],
            sort: Some((PEERS, true)),
        };
        let mut st = ListState::default();
        st.apply(old);
        assert_eq!(
            st.order,
            [SOURCE, 0, 1, 2, 3, 4, 5, UPLOAD, PEERS, RATIO, 6, ADDED, 7]
        );
        assert_eq!(
            st.widths[PEERS], 64.0,
            "Peers keeps the width 0.5.2 saved for it"
        );
        assert_eq!(st.widths[RATIO], 92.0, "Share ratio keeps its own");
        assert_eq!(
            st.widths[UPLOAD], DEFAULT_WIDTHS[UPLOAD],
            "the new slot gets its default"
        );
        assert_eq!(
            st.sort,
            Some((PEERS, true)),
            "a list sorted by Peers is still sorted by Peers"
        );
        assert_eq!(st.widths[SOURCE], 62.0, "the saved widths stay");
        assert_eq!(st.widths[ADDED], 145.0);
    }
}
