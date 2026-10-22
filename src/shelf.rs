use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::book::BookFormat;
use crate::config::ReaderConfig;
use crate::network::{DownloadedBook, NetworkClient, RemoteBook};
use crate::storage::{LibraryBook, ScanDirResult, Store};
use crate::ui::{
    action_line, centered_rect, command_for, fill_background, help_lines, hud_chunks, KeyBinding,
    KeyChord, Palette,
};

pub enum ShelfOutcome {
    Exit,
    Open(LibraryBook),
    Network,
    Local,
}

const PROGRESS_BAR_WIDTH: usize = 5;

pub fn run_shelf(
    store: &mut Store,
    config: &mut ReaderConfig,
    scan_result: Option<ScanDirResult>,
) -> Result<ShelfOutcome> {
    run_shelf_unified(store, config, scan_result, None, None)
}

pub fn run_shelf_with_notice(
    store: &mut Store,
    config: &mut ReaderConfig,
    scan_result: Option<ScanDirResult>,
    notice: Option<String>,
) -> Result<ShelfOutcome> {
    run_shelf_unified(store, config, scan_result, notice, None)
}

pub fn run_shelf_unified(
    store: &mut Store,
    config: &mut ReaderConfig,
    scan_result: Option<ScanDirResult>,
    notice: Option<String>,
    client: Option<NetworkClient>,
) -> Result<ShelfOutcome> {
    let mut terminal = ratatui::init();
    let _guard = TerminalGuard;
    let size = terminal.size()?;
    let viewport = Rect::new(0, 0, size.width, size.height);

    let mut app = ShelfApp::new(store, viewport, scan_result);
    if let Some(notice) = notice {
        app.message = notice;
    }
    if let Some(client) = client {
        app.network_client = Some(client);
        app.sync_network();
    }

    let outcome = loop {
        if let Some(outcome) = app.poll(store, config) {
            break outcome;
        }
        terminal.draw(|frame| draw(frame, &app, store, config))?;
        if !event::poll(std::time::Duration::from_millis(50))? {
            continue;
        }
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if let Some(outcome) = handle_key(key, &mut app, store, config)? {
                    break outcome;
                }
            }
            Event::Resize(width, height) => {
                app.viewport = Rect::new(0, 0, width, height);
            }
            _ => {}
        }
    };
    Ok(outcome)
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShelfSort {
    Recent,
    Title,
    Progress,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShelfFilter {
    #[default]
    All,
    Local,
    Remote,
}

impl ShelfFilter {
    pub fn label(self) -> &'static str {
        match self {
            ShelfFilter::All => "all",
            ShelfFilter::Local => "local",
            ShelfFilter::Remote => "cloud",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShelfMode {
    Browse,
    Search,
    ConfigureBookshelf,
    ImportPath,
    ConfirmDelete,
    ConfirmDeleteFile,
    Help,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BookSource {
    Local,
    Remote(RemoteBook),
}

#[derive(Debug, Clone)]
struct ShelfRow {
    id: String,
    title: String,
    author: Option<String>,
    format: BookFormat,
    progress_tenths: u16,
    last_opened: Option<SystemTime>,
    bookmark_count: usize,
    missing: bool,
    #[allow(dead_code)]
    path: PathBuf,
    source: BookSource,
}

struct ShelfApp {
    books: Vec<ShelfRow>,
    visible: Vec<usize>,
    selection: usize,
    sort: ShelfSort,
    query: String,
    mode: ShelfMode,
    viewport: Rect,
    message: String,
    pending_delete_file: Option<PathBuf>,
    filter: ShelfFilter,
    network_client: Option<NetworkClient>,
    pending_network: Option<Receiver<Result<Vec<RemoteBook>>>>,
    pending_download: Option<Receiver<Result<DownloadedBook>>>,
    is_syncing_network: bool,
}

impl Default for ShelfApp {
    fn default() -> Self {
        Self {
            books: Vec::new(),
            visible: Vec::new(),
            selection: 0,
            sort: ShelfSort::Recent,
            query: String::new(),
            mode: ShelfMode::Browse,
            viewport: Rect::default(),
            message: String::new(),
            pending_delete_file: None,
            filter: ShelfFilter::All,
            network_client: None,
            pending_network: None,
            pending_download: None,
            is_syncing_network: false,
        }
    }
}

impl ShelfApp {
    fn new(store: &Store, viewport: Rect, scan_result: Option<ScanDirResult>) -> Self {
        let mut app = Self {
            books: Vec::new(),
            visible: Vec::new(),
            selection: 0,
            sort: ShelfSort::Recent,
            query: String::new(),
            mode: ShelfMode::Browse,
            viewport,
            message: String::new(),
            pending_delete_file: None,
            filter: ShelfFilter::All,
            network_client: None,
            pending_network: None,
            pending_download: None,
            is_syncing_network: false,
        };
        app.refresh_from_store(store);
        if let Some(result) = scan_result {
            if result.imported > 0 || result.updated > 0 {
                app.message = format!("Imported {} (+{} updated)", result.imported, result.updated);
            } else if !result.errors.is_empty() {
                app.message = format!("{} file(s) failed to load", result.errors.len());
            }
        }
        app
    }

    fn refresh_from_store(&mut self, store: &Store) {
        self.books = store
            .library()
            .books()
            .iter()
            .map(|book| ShelfRow {
                id: book.id.clone(),
                title: book.title.clone(),
                author: book.author.clone(),
                format: book.format,
                progress_tenths: book.progress_tenths(),
                last_opened: book.last_opened,
                bookmark_count: book.bookmarks.len(),
                missing: book.missing,
                path: book.path.clone(),
                source: BookSource::Local,
            })
            .collect();
        self.recompute_visible();
    }

    fn merge_remote_books(&mut self, remote_books: Vec<RemoteBook>, store: &Store) {
        self.refresh_from_store(store);
        let local_books = store.library().books();
        for remote in remote_books {
            let already_local = local_books.iter().any(|b| {
                let same_title = b.title.trim().eq_ignore_ascii_case(remote.title.trim());
                let same_author = match (&b.author, &remote.author) {
                    (Some(a1), Some(a2)) => a1.trim().eq_ignore_ascii_case(a2.trim()),
                    (None, None) => true,
                    _ => false,
                };
                same_title && same_author
            });
            if !already_local {
                self.books.push(ShelfRow {
                    id: remote.id.clone(),
                    title: remote.title.clone(),
                    author: remote.author.clone(),
                    format: remote.format.unwrap_or(BookFormat::Epub),
                    progress_tenths: 0,
                    last_opened: None,
                    bookmark_count: 0,
                    missing: false,
                    path: PathBuf::new(),
                    source: BookSource::Remote(remote),
                });
            }
        }
        self.recompute_visible();
    }

    fn cycle_filter(&mut self) {
        self.filter = match self.filter {
            ShelfFilter::All => ShelfFilter::Local,
            ShelfFilter::Local => ShelfFilter::Remote,
            ShelfFilter::Remote => ShelfFilter::All,
        };
        self.recompute_visible();
        self.message = format!("View: {}", self.filter.label());
    }

    fn sync_network(&mut self) {
        if let Some(client) = &self.network_client {
            let client = client.clone();
            let (tx, rx) = mpsc::channel();
            self.pending_network = Some(rx);
            self.is_syncing_network = true;
            std::thread::spawn(move || {
                let _ = tx.send(client.books());
            });
        }
    }

    fn start_download(&mut self, client: NetworkClient, remote: RemoteBook) {
        let title = remote.title.clone();
        let (tx, rx) = mpsc::channel();
        self.pending_download = Some(rx);
        self.message = format!("Downloading {title}...");
        std::thread::spawn(move || {
            let _ = tx.send(client.download(&remote));
        });
    }

    fn poll(&mut self, store: &mut Store, config: &ReaderConfig) -> Option<ShelfOutcome> {
        if let Some(rx) = self.pending_download.as_ref() {
            match rx.try_recv() {
                Ok(Ok(download)) => {
                    self.pending_download = None;
                    match crate::network::import_download(store, config, &download) {
                        Ok(book) => return Some(ShelfOutcome::Open(book)),
                        Err(err) => self.message = format!("Import failed: {err:#}"),
                    }
                }
                Ok(Err(err)) => {
                    self.pending_download = None;
                    self.message = format!("Download failed: {err:#}");
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending_download = None;
                    self.message = "Download interrupted".into();
                }
            }
        }

        if let Some(rx) = self.pending_network.as_ref() {
            match rx.try_recv() {
                Ok(Ok(remote_books)) => {
                    self.pending_network = None;
                    self.is_syncing_network = false;
                    let count = remote_books.len();
                    self.merge_remote_books(remote_books, store);
                    self.message = format!("Synced {count} network books");
                }
                Ok(Err(err)) => {
                    self.pending_network = None;
                    self.is_syncing_network = false;
                    self.message = format!("Network sync failed: {err:#}");
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending_network = None;
                    self.is_syncing_network = false;
                }
            }
        }
        None
    }

    fn recompute_visible(&mut self) {
        let needle = self.query.trim().to_lowercase();
        let mut indices: Vec<usize> = self
            .books
            .iter()
            .enumerate()
            .filter(|(_, book)| match self.filter {
                ShelfFilter::All => true,
                ShelfFilter::Local => matches!(book.source, BookSource::Local),
                ShelfFilter::Remote => matches!(book.source, BookSource::Remote(_)),
            })
            .filter(|(_, book)| {
                if needle.is_empty() {
                    return true;
                }
                let title = book.title.to_lowercase();
                let author_match = book
                    .author
                    .as_deref()
                    .map(|author| author.to_lowercase().contains(&needle))
                    .unwrap_or(false);
                title.contains(&needle) || author_match
            })
            .map(|(index, _)| index)
            .collect();

        match self.sort {
            ShelfSort::Recent => indices.sort_by(|&a, &b| {
                let at = self.books[a].last_opened;
                let bt = self.books[b].last_opened;
                match (at, bt) {
                    (Some(at), Some(bt)) => bt.cmp(&at),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => self.books[a].title.cmp(&self.books[b].title),
                }
            }),
            ShelfSort::Title => {
                indices.sort_by(|&a, &b| self.books[a].title.cmp(&self.books[b].title));
            }
            ShelfSort::Progress => indices.sort_by(|&a, &b| {
                self.progress_percent(&self.books[b])
                    .partial_cmp(&self.progress_percent(&self.books[a]))
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
        }

        self.visible = indices;
        if self.visible.is_empty() {
            self.selection = 0;
        } else if self.selection >= self.visible.len() {
            self.selection = self.visible.len() - 1;
        }
    }

    fn progress_percent(&self, row: &ShelfRow) -> f64 {
        f64::from(row.progress_tenths) / 10.0
    }

    fn cycle_sort(&mut self) {
        self.sort = match self.sort {
            ShelfSort::Recent => ShelfSort::Title,
            ShelfSort::Title => ShelfSort::Progress,
            ShelfSort::Progress => ShelfSort::Recent,
        };
        self.recompute_visible();
        self.message = format!("Sort: {}", self.sort.label());
    }

    fn selected_row(&self) -> Option<&ShelfRow> {
        self.visible
            .get(self.selection)
            .map(|&index| &self.books[index])
    }

    fn move_selection(&mut self, delta: isize) {
        if self.visible.is_empty() {
            self.selection = 0;
            return;
        }
        let len = self.visible.len() as isize;
        let current = self.selection as isize;
        let next = (current + delta).rem_euclid(len);
        self.selection = next as usize;
    }

    fn select_book(&mut self, book_id: &str) {
        if let Some(selection) = self
            .visible
            .iter()
            .position(|&index| self.books[index].id == book_id)
        {
            self.selection = selection;
        }
    }
}

impl ShelfSort {
    fn label(self) -> &'static str {
        match self {
            ShelfSort::Recent => "recent",
            ShelfSort::Title => "title",
            ShelfSort::Progress => "progress",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShelfCommandUi {
    Open,
    Next,
    Previous,
    First,
    Last,
    PageDown,
    PageUp,
    Search,
    Import,
    Sort,
    Delete,
    Resume,
    Help,
    Quit,
    Network,
    SyncNetwork,
    CycleTheme,
}

const OPEN_KEYS: &[KeyChord] = &[KeyChord::plain(KeyCode::Enter)];
const NEXT_KEYS: &[KeyChord] = &[
    KeyChord::plain(KeyCode::Char('j')),
    KeyChord::plain(KeyCode::Down),
];
const PREVIOUS_KEYS: &[KeyChord] = &[
    KeyChord::plain(KeyCode::Char('k')),
    KeyChord::plain(KeyCode::Up),
];
const FIRST_KEYS: &[KeyChord] = &[KeyChord::plain(KeyCode::Char('g'))];
const LAST_KEYS: &[KeyChord] = &[KeyChord::plain(KeyCode::Char('G'))];
const PAGE_DOWN_KEYS: &[KeyChord] = &[KeyChord::plain(KeyCode::PageDown)];
const PAGE_UP_KEYS: &[KeyChord] = &[KeyChord::plain(KeyCode::PageUp)];
const SEARCH_KEYS: &[KeyChord] = &[KeyChord::plain(KeyCode::Char('/'))];
const IMPORT_KEYS: &[KeyChord] = &[KeyChord::plain(KeyCode::Char('i'))];
const SORT_KEYS: &[KeyChord] = &[KeyChord::plain(KeyCode::Char('s'))];
const DELETE_KEYS: &[KeyChord] = &[KeyChord::plain(KeyCode::Char('d'))];
const RESUME_KEYS: &[KeyChord] = &[KeyChord::plain(KeyCode::Char('r'))];
const HELP_KEYS: &[KeyChord] = &[
    KeyChord::plain(KeyCode::Char('?')),
    KeyChord::plain(KeyCode::F(1)),
];
const QUIT_KEYS: &[KeyChord] = &[
    KeyChord::plain(KeyCode::Char('q')),
    KeyChord::plain(KeyCode::Esc),
    KeyChord::control('c'),
];

const SHELF_BINDINGS: &[KeyBinding<ShelfCommandUi>] = &[
    KeyBinding {
        command: ShelfCommandUi::Network,
        chords: &[KeyChord::plain(KeyCode::Char('w'))],
        keys: "w",
        hud_keys: "w",
        label: "Filter network / local",
        short_label: "View",
        show_in_hud: true,
        hud_priority: 0,
    },
    KeyBinding {
        command: ShelfCommandUi::SyncNetwork,
        chords: &[KeyChord::plain(KeyCode::Char('R'))],
        keys: "R",
        hud_keys: "R",
        label: "Sync network bookshelf",
        short_label: "Sync",
        show_in_hud: true,
        hud_priority: 1,
    },
    KeyBinding {
        command: ShelfCommandUi::Open,
        chords: OPEN_KEYS,
        keys: "Enter",
        hud_keys: "Enter",
        label: "Open selected book",
        short_label: "Open",
        show_in_hud: true,
        hud_priority: 0,
    },
    KeyBinding {
        command: ShelfCommandUi::Next,
        chords: NEXT_KEYS,
        keys: "j / Down",
        hud_keys: "Down",
        label: "Next book",
        short_label: "Next",
        show_in_hud: true,
        hud_priority: 0,
    },
    KeyBinding {
        command: ShelfCommandUi::Previous,
        chords: PREVIOUS_KEYS,
        keys: "k / Up",
        hud_keys: "Up",
        label: "Previous book",
        short_label: "Prev",
        show_in_hud: true,
        hud_priority: 0,
    },
    KeyBinding {
        command: ShelfCommandUi::Search,
        chords: SEARCH_KEYS,
        keys: "/",
        hud_keys: "/",
        label: "Filter title or author",
        short_label: "Search",
        show_in_hud: true,
        hud_priority: 0,
    },
    KeyBinding {
        command: ShelfCommandUi::Import,
        chords: IMPORT_KEYS,
        keys: "i",
        hud_keys: "i",
        label: "Import a book",
        short_label: "Import",
        show_in_hud: true,
        hud_priority: 0,
    },
    KeyBinding {
        command: ShelfCommandUi::Sort,
        chords: SORT_KEYS,
        keys: "s",
        hud_keys: "s",
        label: "Cycle sort order",
        short_label: "Sort",
        show_in_hud: true,
        hud_priority: 1,
    },
    KeyBinding {
        command: ShelfCommandUi::Delete,
        chords: DELETE_KEYS,
        keys: "d",
        hud_keys: "d",
        label: "Remove selected book",
        short_label: "Delete",
        show_in_hud: true,
        hud_priority: 1,
    },
    KeyBinding {
        command: ShelfCommandUi::Resume,
        chords: RESUME_KEYS,
        keys: "r",
        hud_keys: "r",
        label: "Resume most recent book",
        short_label: "Resume",
        show_in_hud: false,
        hud_priority: 2,
    },
    KeyBinding {
        command: ShelfCommandUi::CycleTheme,
        chords: &[KeyChord::plain(KeyCode::Char('C'))],
        keys: "C",
        hud_keys: "C",
        label: "Cycle theme / skin",
        short_label: "Skin",
        show_in_hud: true,
        hud_priority: 1,
    },
    KeyBinding {
        command: ShelfCommandUi::Help,
        chords: HELP_KEYS,
        keys: "? / F1",
        hud_keys: "?",
        label: "Full help",
        short_label: "Help",
        show_in_hud: true,
        hud_priority: 0,
    },
    KeyBinding {
        command: ShelfCommandUi::Quit,
        chords: QUIT_KEYS,
        keys: "q / Esc / Ctrl+C",
        hud_keys: "q",
        label: "Quit shelf",
        short_label: "Quit",
        show_in_hud: true,
        hud_priority: 0,
    },
    KeyBinding {
        command: ShelfCommandUi::First,
        chords: FIRST_KEYS,
        keys: "g",
        hud_keys: "g",
        label: "First book",
        short_label: "First",
        show_in_hud: false,
        hud_priority: 2,
    },
    KeyBinding {
        command: ShelfCommandUi::Last,
        chords: LAST_KEYS,
        keys: "G",
        hud_keys: "G",
        label: "Last book",
        short_label: "Last",
        show_in_hud: false,
        hud_priority: 2,
    },
    KeyBinding {
        command: ShelfCommandUi::PageDown,
        chords: PAGE_DOWN_KEYS,
        keys: "PageDown",
        hud_keys: "PgDn",
        label: "Move down ten books",
        short_label: "Down 10",
        show_in_hud: false,
        hud_priority: 2,
    },
    KeyBinding {
        command: ShelfCommandUi::PageUp,
        chords: PAGE_UP_KEYS,
        keys: "PageUp",
        hud_keys: "PgUp",
        label: "Move up ten books",
        short_label: "Up 10",
        show_in_hud: false,
        hud_priority: 2,
    },
];

fn handle_key(
    key: KeyEvent,
    app: &mut ShelfApp,
    store: &mut Store,
    config: &mut ReaderConfig,
) -> Result<Option<ShelfOutcome>> {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Ok(Some(ShelfOutcome::Exit));
    }

    match app.mode {
        ShelfMode::Browse => handle_browse_key(key, app, store, config),
        ShelfMode::Search => handle_search_key(key, app),
        ShelfMode::ConfigureBookshelf => handle_configure_bookshelf_key(key, app, store, config),
        ShelfMode::ImportPath => handle_import_path_key(key, app, store, config),
        ShelfMode::ConfirmDelete => handle_confirm_delete_key(key, app, store, config),
        ShelfMode::ConfirmDeleteFile => handle_confirm_delete_file_key(key, app),
        ShelfMode::Help => {
            if matches!(
                key.code,
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('?') | KeyCode::F(1)
            ) {
                app.mode = ShelfMode::Browse;
            }
            Ok(None)
        }
    }
}

fn handle_browse_key(
    key: KeyEvent,
    app: &mut ShelfApp,
    store: &mut Store,
    config: &mut ReaderConfig,
) -> Result<Option<ShelfOutcome>> {
    match command_for(SHELF_BINDINGS, key) {
        Some(ShelfCommandUi::Network) => {
            if app.network_client.is_some() {
                app.cycle_filter();
                return Ok(None);
            } else {
                return Ok(Some(ShelfOutcome::Network));
            }
        }
        Some(ShelfCommandUi::SyncNetwork) => {
            if app.network_client.is_some() {
                app.sync_network();
                app.message = "Syncing network books...".into();
            } else {
                app.message = "Network not connected. Press w to connect.".into();
            }
            return Ok(None);
        }
        Some(ShelfCommandUi::Quit) => return Ok(Some(ShelfOutcome::Exit)),
        Some(ShelfCommandUi::Next) => app.move_selection(1),
        Some(ShelfCommandUi::Previous) => app.move_selection(-1),
        Some(ShelfCommandUi::First) => app.selection = 0,
        Some(ShelfCommandUi::Last) if !app.visible.is_empty() => {
            app.selection = app.visible.len() - 1;
        }
        Some(ShelfCommandUi::PageDown) => app.move_selection(10),
        Some(ShelfCommandUi::PageUp) => app.move_selection(-10),
        Some(ShelfCommandUi::Search) => {
            app.mode = ShelfMode::Search;
            app.query.clear();
        }
        Some(ShelfCommandUi::Import) => {
            app.query.clear();
            app.recompute_visible();
            app.mode = if config.bookshelf_dir.is_some() {
                ShelfMode::ImportPath
            } else {
                ShelfMode::ConfigureBookshelf
            };
            app.message = if config.bookshelf_dir.is_some() {
                "Paste or type a book path".to_string()
            } else {
                "Set a managed bookshelf directory first".to_string()
            };
        }
        Some(ShelfCommandUi::Sort) => app.cycle_sort(),
        Some(ShelfCommandUi::Help) => app.mode = ShelfMode::Help,
        Some(ShelfCommandUi::CycleTheme) => {
            let new_theme = config.cycle_reading_theme();
            let _ = config.save();
            app.message = format!("Skin: {}", new_theme.label());
            return Ok(None);
        }
        Some(ShelfCommandUi::Delete) => {
            if let Some(row) = app.selected_row() {
                if matches!(row.source, BookSource::Remote(_)) {
                    app.message = "Cannot delete a remote book".to_string();
                    return Ok(None);
                }
                let title = row.title.clone();
                app.mode = ShelfMode::ConfirmDelete;
                app.message = format!("Delete {:?}? (y/n)", title);
            } else {
                app.message = "Nothing to delete".to_string();
            }
        }
        Some(ShelfCommandUi::Resume) => {
            let Some(id) = store.library().last_book_id.as_deref() else {
                app.message = "No recent book to resume".to_string();
                return Ok(None);
            };
            let Some(entry) = store.library().books().iter().find(|b| b.id == id).cloned() else {
                app.message = "Recent book not found in library".to_string();
                return Ok(None);
            };
            if entry.missing {
                app.message = "Recent book is missing. Restore it under bookshelf_dir and refresh."
                    .to_string();
                return Ok(None);
            }
            return Ok(Some(ShelfOutcome::Open(entry)));
        }
        Some(ShelfCommandUi::Open) => {
            let Some(row) = app.selected_row() else {
                app.message = "No book selected".to_string();
                return Ok(None);
            };
            match &row.source {
                BookSource::Local => {
                    let id = row.id.clone();
                    if row.missing {
                        app.message =
                            "Book file is missing. Restore it under bookshelf_dir and refresh."
                                .to_string();
                        return Ok(None);
                    }
                    let Some(entry) = store.library().books().iter().find(|b| b.id == id).cloned()
                    else {
                        bail!("Selected book disappeared from library");
                    };
                    return Ok(Some(ShelfOutcome::Open(entry)));
                }
                BookSource::Remote(remote) => {
                    if app.pending_download.is_some() {
                        app.message = "Download already in progress...".to_string();
                        return Ok(None);
                    }
                    if let Some(client) = &app.network_client {
                        let client = client.clone();
                        let remote = remote.clone();
                        app.start_download(client, remote);
                    } else {
                        app.message = "Network not connected".to_string();
                    }
                    return Ok(None);
                }
            }
        }
        _ => {}
    }
    Ok(None)
}

fn handle_search_key(key: KeyEvent, app: &mut ShelfApp) -> Result<Option<ShelfOutcome>> {
    match key.code {
        KeyCode::Esc => {
            app.mode = ShelfMode::Browse;
            app.query.clear();
            app.recompute_visible();
            app.message.clear();
        }
        KeyCode::Enter => {
            app.mode = ShelfMode::Browse;
            app.recompute_visible();
            app.message = if app.query.is_empty() {
                String::new()
            } else {
                format!("Filter: {:?}", app.query)
            };
        }
        KeyCode::Backspace => {
            app.query.pop();
        }
        KeyCode::Char(ch) => app.query.push(ch),
        _ => {}
    }
    Ok(None)
}

fn handle_configure_bookshelf_key(
    key: KeyEvent,
    app: &mut ShelfApp,
    store: &mut Store,
    config: &mut ReaderConfig,
) -> Result<Option<ShelfOutcome>> {
    match key.code {
        KeyCode::Esc => cancel_path_prompt(app, "Bookshelf setup cancelled"),
        KeyCode::Backspace => {
            app.query.pop();
        }
        KeyCode::Enter => {
            let result = (|| -> Result<(PathBuf, crate::bookshelf::RefreshReport)> {
                let input = crate::bookshelf::path_from_user_input(&app.query)?;
                let directory = crate::bookshelf::resolve_bookshelf_dir(&input)?;
                let mut staged_store = store.clone();
                let mut staged_config = config.clone();
                let report = crate::bookshelf::refresh(&mut staged_store, &directory)?;
                staged_config.bookshelf_dir = Some(directory.clone());
                staged_config.save()?;
                staged_store.save()?;
                *store = staged_store;
                *config = staged_config;
                Ok((directory, report))
            })();

            match result {
                Ok((directory, report)) => {
                    app.refresh_from_store(store);
                    app.query.clear();
                    app.mode = ShelfMode::ImportPath;
                    app.message = if report.errors.is_empty() {
                        format!("Bookshelf set: {}. Enter a book path", directory.display())
                    } else {
                        format!(
                            "Bookshelf set with {} refresh error(s); enter a book path",
                            report.errors.len()
                        )
                    };
                }
                Err(err) => app.message = format!("Bookshelf setup failed: {err}"),
            }
        }
        KeyCode::Char(ch) => app.query.push(ch),
        _ => {}
    }
    Ok(None)
}

fn handle_import_path_key(
    key: KeyEvent,
    app: &mut ShelfApp,
    store: &mut Store,
    config: &ReaderConfig,
) -> Result<Option<ShelfOutcome>> {
    match key.code {
        KeyCode::Esc => cancel_path_prompt(app, "Import cancelled"),
        KeyCode::Backspace => {
            app.query.pop();
        }
        KeyCode::Enter => {
            let result = crate::bookshelf::path_from_user_input(&app.query)
                .and_then(|source| crate::bookshelf::import_managed_book(store, config, &source));
            match result {
                Ok(outcome) => {
                    let warning_count = outcome.warnings.len();
                    let book_id = outcome.book.id.clone();
                    let verb = match outcome.disposition {
                        crate::bookshelf::ImportDisposition::Copied => "Imported",
                        crate::bookshelf::ImportDisposition::Reused => "Already imported",
                    };
                    app.query.clear();
                    app.mode = ShelfMode::Browse;
                    app.refresh_from_store(store);
                    app.select_book(&book_id);
                    app.message = format!(
                        "{verb}: {} -> {}{}",
                        outcome.book.title,
                        outcome.book.path.display(),
                        if warning_count == 0 {
                            String::new()
                        } else {
                            format!(" ({warning_count} warning(s))")
                        }
                    );
                }
                Err(err) => app.message = format!("Import failed: {err}"),
            }
        }
        KeyCode::Char(ch) => app.query.push(ch),
        _ => {}
    }
    Ok(None)
}

fn cancel_path_prompt(app: &mut ShelfApp, message: &str) {
    app.mode = ShelfMode::Browse;
    app.query.clear();
    app.recompute_visible();
    app.message = message.to_string();
}

fn handle_confirm_delete_key(
    key: KeyEvent,
    app: &mut ShelfApp,
    store: &mut Store,
    config: &ReaderConfig,
) -> Result<Option<ShelfOutcome>> {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
            if let Some(row) = app.selected_row() {
                let id = row.id.clone();
                let title = row.title.clone();
                let path = row.path.clone();
                match store.remove_book(&id) {
                    Ok(removed) => {
                        crate::bookshelf::remove_sidecar_for_book(&removed)?;
                        store.save()?;
                        app.refresh_from_store(store);
                        if should_prompt_delete_file(&path, config) {
                            app.pending_delete_file = Some(path);
                            app.mode = ShelfMode::ConfirmDeleteFile;
                            app.message = format!("Also delete file for {title:?}? (y/n)");
                            return Ok(None);
                        }
                        app.message = format!("Removed: {title}");
                    }
                    Err(err) => {
                        app.mode = ShelfMode::Browse;
                        app.message = format!("Delete failed: {err}");
                        return Ok(None);
                    }
                }
            }
            app.mode = ShelfMode::Browse;
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            app.mode = ShelfMode::Browse;
            app.message = "Delete cancelled".to_string();
        }
        _ => {}
    }
    Ok(None)
}

fn handle_confirm_delete_file_key(
    key: KeyEvent,
    app: &mut ShelfApp,
) -> Result<Option<ShelfOutcome>> {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
            if let Some(path) = app.pending_delete_file.take() {
                if path.exists() {
                    std::fs::remove_file(&path)
                        .with_context(|| format!("Failed to delete {}", path.display()))?;
                }
                app.message = "Removed from library and deleted file".to_string();
            }
            app.mode = ShelfMode::Browse;
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            app.pending_delete_file = None;
            app.mode = ShelfMode::Browse;
            app.message = "Removed from library; file kept".to_string();
        }
        _ => {}
    }
    Ok(None)
}

fn draw(frame: &mut ratatui::Frame<'_>, app: &ShelfApp, store: &Store, config: &ReaderConfig) {
    let palette = Palette::for_reading_theme(config.reading_theme);
    fill_background(frame, palette);
    let [status_area, body_area, action_area] = hud_chunks(app.viewport);

    let status = match app.mode {
        ShelfMode::Search => Line::from(vec![
            Span::styled("Search: /", palette.accent()),
            Span::styled(app.query.clone(), palette.body()),
        ]),
        ShelfMode::ConfigureBookshelf => Line::from(vec![
            Span::styled("Bookshelf: ", palette.accent()),
            Span::styled(app.query.clone(), palette.body()),
        ]),
        ShelfMode::ImportPath => Line::from(vec![
            Span::styled("Import: ", palette.accent()),
            Span::styled(app.query.clone(), palette.body()),
        ]),
        _ => {
            let filter_info = match app.filter {
                ShelfFilter::All => "",
                ShelfFilter::Local => "  ·  view: local",
                ShelfFilter::Remote => "  ·  view: cloud",
            };
            let sync_info = if app.is_syncing_network {
                "  ·  syncing..."
            } else {
                ""
            };
            Line::from(vec![
                Span::styled("My Library", palette.accent()),
                Span::styled(
                    format!(
                        "  {} of {} books  ·  sort: {}{filter_info}{sync_info}",
                        app.visible.len(),
                        app.books.len(),
                        app.sort.label()
                    ),
                    palette.body(),
                ),
            ])
        }
    };
    frame.render_widget(Paragraph::new(status).style(palette.base()), status_area);

    let inner = Block::default().borders(Borders::NONE).inner(body_area);
    if app.books.is_empty() {
        frame.render_widget(
            Paragraph::new("No books in library. Press `i` or use `reader import <file>`.")
                .style(palette.muted())
                .wrap(Wrap { trim: false }),
            inner,
        );
    } else if app.visible.is_empty() {
        frame.render_widget(
            Paragraph::new(format!("No matches for {:?}", app.query)).style(palette.muted()),
            inner,
        );
    } else {
        let last_book_id = store.library().last_book_id.as_deref();
        let max_items = inner.height as usize;
        let scroll = app.selection.saturating_sub(max_items.saturating_sub(1));
        let lines: Vec<Line> = app
            .visible
            .iter()
            .skip(scroll)
            .take(max_items)
            .enumerate()
            .map(|(offset, &index)| {
                let row = &app.books[index];
                let real_index = scroll + offset;
                build_row_line(row, real_index == app.selection, last_book_id, palette)
            })
            .collect();
        frame.render_widget(Paragraph::new(lines).style(palette.body()), inner);
    }

    let actions = match app.mode {
        ShelfMode::Browse => action_line(SHELF_BINDINGS, app.viewport.width, palette),
        ShelfMode::Search => shelf_prompt_actions(
            &[("Enter", "Apply"), ("Backspace", "Edit"), ("Esc", "Cancel")],
            palette,
        ),
        ShelfMode::ConfigureBookshelf | ShelfMode::ImportPath => shelf_prompt_actions(
            &[
                ("Enter", "Confirm"),
                ("Backspace", "Edit"),
                ("Esc", "Cancel"),
            ],
            palette,
        ),
        ShelfMode::ConfirmDelete | ShelfMode::ConfirmDeleteFile => {
            shelf_prompt_actions(&[("y/Enter", "Confirm"), ("n/Esc", "Cancel")], palette)
        }
        ShelfMode::Help => shelf_prompt_actions(&[("Esc/Enter", "Close")], palette),
    };
    frame.render_widget(Paragraph::new(actions).style(palette.base()), action_area);

    if !app.message.is_empty() && body_area.height > 0 {
        let width = (app.message.chars().count() as u16 + 4).min(body_area.width.max(1));
        let area = Rect::new(
            body_area.right().saturating_sub(width),
            body_area.bottom().saturating_sub(1),
            width,
            1,
        );
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(format!("  {}  ", app.message)).style(palette.accent()),
            area,
        );
    }

    if app.mode == ShelfMode::Help {
        draw_shelf_help(frame, palette);
    }
}

fn build_row_line(
    row: &ShelfRow,
    selected: bool,
    last_book_id: Option<&str>,
    palette: Palette,
) -> Line<'static> {
    let marker = if last_book_id == Some(row.id.as_str()) {
        "*"
    } else {
        " "
    };
    let prefix = if selected { "> " } else { "  " };
    let title = fit_column(&row.title, 30);
    let author = row
        .author
        .as_deref()
        .map(|value| fit_column(value, 16))
        .unwrap_or_else(|| fit_column("-", 16));
    let percent = compute_percent(row);
    let last = row
        .last_opened
        .map(format_relative_time)
        .unwrap_or_else(|| "-".to_string());
    let bookmarks = if row.bookmark_count == 0 {
        "-".to_string()
    } else {
        format!("{}bm", row.bookmark_count)
    };
    let (status, status_style) = match &row.source {
        BookSource::Local => {
            if row.missing {
                ("MISS", palette.error())
            } else {
                ("OK", palette.success())
            }
        }
        BookSource::Remote(_) => ("NET", palette.accent()),
    };
    let format = row.format.label();

    if selected {
        let bar = progress_bar(percent, PROGRESS_BAR_WIDTH);
        let text = format!(
            "{prefix}{marker} {title}  {author}  {bar} {percent:>3}%  {last:>10}  {bookmarks:>4}  {format} {status}"
        );
        Line::from(Span::styled(text, palette.selected()))
    } else {
        let filled = (percent as usize * PROGRESS_BAR_WIDTH + 50) / 100;
        let filled = filled.min(PROGRESS_BAR_WIDTH);
        let empty = PROGRESS_BAR_WIDTH.saturating_sub(filled);

        let mut spans = Vec::with_capacity(16);
        spans.push(Span::styled(prefix, palette.muted()));
        spans.push(Span::styled(marker, palette.accent()));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(title, palette.body()));
        spans.push(Span::raw("  "));
        spans.push(Span::styled(author, palette.muted()));
        spans.push(Span::raw("  "));

        if filled > 0 {
            let filled_style = if percent >= 100 {
                palette.success()
            } else {
                palette.accent()
            };
            spans.push(Span::styled("\u{2588}".repeat(filled), filled_style));
        }
        if empty > 0 {
            spans.push(Span::styled("\u{2591}".repeat(empty), palette.muted()));
        }

        spans.push(Span::raw(" "));
        let percent_style = if percent >= 100 {
            palette.success()
        } else if percent > 0 {
            palette.accent()
        } else {
            palette.muted()
        };
        spans.push(Span::styled(format!("{percent:>3}%"), percent_style));
        spans.push(Span::raw("  "));
        spans.push(Span::styled(format!("{last:>10}"), palette.muted()));
        spans.push(Span::raw("  "));
        let bkmk_style = if row.bookmark_count > 0 {
            palette.accent()
        } else {
            palette.muted()
        };
        spans.push(Span::styled(format!("{bookmarks:>4}"), bkmk_style));
        spans.push(Span::raw("  "));
        spans.push(Span::styled(format, palette.muted()));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(status, status_style));

        Line::from(spans)
    }
}

fn compute_percent(row: &ShelfRow) -> u16 {
    (row.progress_tenths + 5) / 10
}

fn shelf_prompt_actions(actions: &[(&str, &str)], palette: Palette) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, (keys, label)) in actions.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled("  ", palette.muted()));
        }
        spans.push(Span::styled(format!("[{keys}]"), palette.accent()));
        spans.push(Span::styled(format!(" {label}"), palette.muted()));
    }
    Line::from(spans)
}

fn draw_shelf_help(frame: &mut ratatui::Frame<'_>, palette: Palette) {
    let area = centered_rect(72, 82, frame.area());
    frame.render_widget(Clear, area);
    let lines = help_lines("Shelf", SHELF_BINDINGS, palette);
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Shelf help ")
                    .borders(Borders::ALL)
                    .border_style(palette.border())
                    .style(palette.popup()),
            )
            .style(palette.popup())
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn progress_bar(percent: u16, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let filled = (percent as usize * width + 50) / 100;
    let filled = filled.min(width);
    let mut out = String::with_capacity(width * 3 + 1);
    for _ in 0..filled {
        out.push('\u{2588}');
    }
    for _ in filled..width {
        out.push('\u{2591}');
    }
    out
}

fn truncate(text: &str, max: usize) -> String {
    if text.width() <= max {
        text.to_string()
    } else if max == 0 {
        String::new()
    } else if max == 1 {
        "\u{2026}".to_string()
    } else {
        let mut out = String::new();
        for ch in text.chars() {
            let mut candidate = out.clone();
            candidate.push(ch);
            candidate.push('\u{2026}');
            if candidate.width() > max {
                break;
            }
            out.push(ch);
        }
        out.push('\u{2026}');
        out
    }
}

fn fit_column(text: &str, width: usize) -> String {
    let mut fitted = truncate(text, width);
    fitted.push_str(&" ".repeat(width.saturating_sub(fitted.width())));
    fitted
}

fn should_prompt_delete_file(path: &Path, config: &ReaderConfig) -> bool {
    let Some(dir) = config.bookshelf_dir.as_deref() else {
        return false;
    };
    if !path.exists() {
        return false;
    }
    if let (Ok(path), Ok(dir)) = (path.canonicalize(), dir.canonicalize()) {
        path.starts_with(dir)
    } else {
        path.starts_with(dir)
    }
}

pub fn format_relative_time(time: SystemTime) -> String {
    let Ok(duration) = SystemTime::now().duration_since(time) else {
        return "future".to_string();
    };
    let secs = duration.as_secs();
    if secs < 60 {
        return "just now".to_string();
    }
    let minutes = secs / 60;
    if minutes < 60 {
        return format!("{minutes}m");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours}h");
    }
    let days = hours / 24;
    if days < 7 {
        return format!("{days}d");
    }
    let epoch_secs = time
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days_since_epoch = epoch_secs / 86_400;
    let (year, month, day) = epoch_days_to_ymd(days_since_epoch);
    format!("{year:04}-{month:02}-{day:02}")
}

fn epoch_days_to_ymd(mut days: i64) -> (i32, u32, u32) {
    // Howard Hinnant's days_from_civil algorithm.
    days += 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let doe = (days - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    fn row(id: &str, title: &str, last: Option<SystemTime>) -> ShelfRow {
        ShelfRow {
            id: id.into(),
            title: title.into(),
            author: None,
            format: BookFormat::Txt,
            progress_tenths: 0,
            last_opened: last,
            bookmark_count: 0,
            path: PathBuf::from(format!("/tmp/{id}.txt")),
            missing: false,
            source: BookSource::Local,
        }
    }

    #[test]
    fn removed_theme_key_is_not_a_shelf_command() {
        assert_eq!(
            command_for(
                SHELF_BINDINGS,
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE)
            ),
            None
        );
    }

    #[test]
    fn import_key_is_declared_once_in_shelf_bindings() {
        assert_eq!(
            command_for(
                SHELF_BINDINGS,
                KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE)
            ),
            Some(ShelfCommandUi::Import)
        );
    }

    #[test]
    fn network_toggle_returns_to_dispatch_without_modifying_local_library() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("library.json")).unwrap();
        let mut config = ReaderConfig::default();
        let mut app = ShelfApp::new(&store, Rect::new(0, 0, 80, 24), None);
        let outcome = handle_key(
            KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE),
            &mut app,
            &mut store,
            &mut config,
        )
        .unwrap();
        assert!(matches!(outcome, Some(ShelfOutcome::Network)));
        assert!(store.library().books().is_empty());
        assert!(config.bookshelf_dir.is_none());
    }

    #[test]
    fn import_key_guides_unconfigured_shelf_then_accepts_book_path() {
        let dir = tempfile::tempdir().unwrap();
        let shelf = dir.path().join("shelf");
        std::fs::create_dir(&shelf).unwrap();
        let mut store = Store::open(dir.path().join("library.json")).unwrap();
        let mut config = ReaderConfig::open(dir.path().join("configuration.json")).unwrap();
        let mut app = ShelfApp::new(&store, Rect::new(0, 0, 80, 24), None);

        handle_key(
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
            &mut app,
            &mut store,
            &mut config,
        )
        .unwrap();
        assert_eq!(app.mode, ShelfMode::ConfigureBookshelf);

        app.query = format!("\"{}\"", shelf.display());
        handle_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut app,
            &mut store,
            &mut config,
        )
        .unwrap();

        assert_eq!(app.mode, ShelfMode::ImportPath);
        assert_eq!(
            config
                .bookshelf_dir
                .as_deref()
                .unwrap()
                .canonicalize()
                .unwrap(),
            shelf.canonicalize().unwrap()
        );
        assert!(app.query.is_empty());
    }

    #[test]
    fn shelf_import_pastes_quoted_unicode_path_and_selects_new_book() {
        let dir = tempfile::tempdir().unwrap();
        let source_dir = dir.path().join("downloads");
        let shelf = dir.path().join("shelf");
        std::fs::create_dir(&source_dir).unwrap();
        std::fs::create_dir(&shelf).unwrap();
        let source = source_dir.join("一地鸡毛.txt");
        std::fs::write(&source, "刘震云").unwrap();
        let mut store = Store::open(dir.path().join("library.json")).unwrap();
        let mut config = ReaderConfig::default();
        config.bookshelf_dir = Some(shelf.clone());
        let mut app = ShelfApp::new(&store, Rect::new(0, 0, 80, 24), None);
        app.mode = ShelfMode::ImportPath;
        app.query = format!("\"{}\"", source.display());

        handle_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut app,
            &mut store,
            &mut config,
        )
        .unwrap();

        assert_eq!(app.mode, ShelfMode::Browse);
        assert_eq!(app.selected_row().unwrap().title, "一地鸡毛");
        assert!(shelf.join("一地鸡毛.txt").exists());
        assert!(app.message.starts_with("Imported:"));
    }

    #[test]
    fn shelf_import_error_stays_in_path_prompt_and_escape_cancels() {
        let dir = tempfile::tempdir().unwrap();
        let shelf = dir.path().join("shelf");
        std::fs::create_dir(&shelf).unwrap();
        let mut store = Store::open(dir.path().join("library.json")).unwrap();
        let mut config = ReaderConfig::default();
        config.bookshelf_dir = Some(shelf);
        let mut app = ShelfApp::new(&store, Rect::new(0, 0, 80, 24), None);
        app.mode = ShelfMode::ImportPath;
        app.query = "missing.epub".into();

        handle_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut app,
            &mut store,
            &mut config,
        )
        .unwrap();
        assert_eq!(app.mode, ShelfMode::ImportPath);
        assert!(app.message.starts_with("Import failed:"));

        handle_key(
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            &mut app,
            &mut store,
            &mut config,
        )
        .unwrap();
        assert_eq!(app.mode, ShelfMode::Browse);
        assert!(app.query.is_empty());
    }

    #[test]
    fn recompute_visible_keeps_books_when_query_empty() {
        let mut app = ShelfApp {
            books: vec![row("a", "Alpha", None), row("b", "Beta", None)],
            visible: vec![],
            selection: 0,
            sort: ShelfSort::Recent,
            query: String::new(),
            mode: ShelfMode::Browse,
            viewport: Rect::new(0, 0, 80, 24),
            message: String::new(),
            pending_delete_file: None,
            filter: ShelfFilter::All,
            network_client: None,
            pending_network: None,
            pending_download: None,
            is_syncing_network: false,
        };
        app.recompute_visible();
        assert_eq!(app.visible.len(), 2);
    }

    #[test]
    fn search_filter_matches_title_and_author() {
        let mut app = ShelfApp {
            books: vec![row("a", "Rust Programming", None), row("b", "Other", None)],
            visible: vec![],
            selection: 0,
            sort: ShelfSort::Title,
            query: String::new(),
            mode: ShelfMode::Search,
            viewport: Rect::new(0, 0, 80, 24),
            message: String::new(),
            pending_delete_file: None,
            filter: ShelfFilter::All,
            network_client: None,
            pending_network: None,
            pending_download: None,
            is_syncing_network: false,
        };
        app.books[1].author = Some("Rustacean".into());
        app.query = "rust".into();
        app.recompute_visible();
        let titles: Vec<&str> = app
            .visible
            .iter()
            .map(|&i| app.books[i].title.as_str())
            .collect();
        assert!(titles.contains(&"Rust Programming"));
        assert!(titles.contains(&"Other"));
    }

    #[test]
    fn sort_recent_puts_most_recent_first_and_unopened_last() {
        let now = SystemTime::now();
        let mut app = ShelfApp {
            books: vec![
                row("a", "Alpha", None),
                row("b", "Beta", Some(now - Duration::from_secs(7200))),
                row("c", "Gamma", Some(now)),
            ],
            visible: vec![],
            selection: 0,
            sort: ShelfSort::Recent,
            query: String::new(),
            mode: ShelfMode::Browse,
            viewport: Rect::new(0, 0, 80, 24),
            message: String::new(),
            pending_delete_file: None,
            filter: ShelfFilter::All,
            network_client: None,
            pending_network: None,
            pending_download: None,
            is_syncing_network: false,
        };
        app.recompute_visible();
        assert_eq!(app.books[app.visible[0]].id, "c");
        assert_eq!(app.books[app.visible[1]].id, "b");
        assert_eq!(app.books[app.visible[2]].id, "a");
    }

    #[test]
    fn sort_title_uses_alphabetical_order() {
        let mut app = ShelfApp {
            books: vec![
                row("a", "Charlie", None),
                row("b", "Alpha", None),
                row("c", "Bravo", None),
            ],
            visible: vec![],
            selection: 0,
            sort: ShelfSort::Title,
            query: String::new(),
            mode: ShelfMode::Browse,
            viewport: Rect::new(0, 0, 80, 24),
            message: String::new(),
            pending_delete_file: None,
            filter: ShelfFilter::All,
            network_client: None,
            pending_network: None,
            pending_download: None,
            is_syncing_network: false,
        };
        app.recompute_visible();
        let titles: Vec<&str> = app
            .visible
            .iter()
            .map(|&i| app.books[i].title.as_str())
            .collect();
        assert_eq!(titles, vec!["Alpha", "Bravo", "Charlie"]);
    }

    #[test]
    fn sort_progress_orders_high_to_low() {
        let mut app = ShelfApp {
            books: vec![
                row("a", "Half", None),
                row("b", "Quarter", None),
                row("c", "Full", None),
            ],
            visible: vec![],
            selection: 0,
            sort: ShelfSort::Progress,
            query: String::new(),
            mode: ShelfMode::Browse,
            viewport: Rect::new(0, 0, 80, 24),
            message: String::new(),
            pending_delete_file: None,
            filter: ShelfFilter::All,
            network_client: None,
            pending_network: None,
            pending_download: None,
            is_syncing_network: false,
        };
        app.books[0].progress_tenths = 500;
        app.books[1].progress_tenths = 250;
        app.books[2].progress_tenths = 1000;
        app.recompute_visible();
        let titles: Vec<&str> = app
            .visible
            .iter()
            .map(|&i| app.books[i].title.as_str())
            .collect();
        assert_eq!(titles, vec!["Full", "Half", "Quarter"]);
    }

    #[test]
    fn cycle_sort_walks_through_modes() {
        let mut app = ShelfApp {
            books: vec![row("a", "Alpha", None)],
            visible: vec![],
            selection: 0,
            sort: ShelfSort::Recent,
            query: String::new(),
            mode: ShelfMode::Browse,
            viewport: Rect::new(0, 0, 80, 24),
            message: String::new(),
            pending_delete_file: None,
            filter: ShelfFilter::All,
            network_client: None,
            pending_network: None,
            pending_download: None,
            is_syncing_network: false,
        };
        app.cycle_sort();
        assert_eq!(app.sort, ShelfSort::Title);
        app.cycle_sort();
        assert_eq!(app.sort, ShelfSort::Progress);
        app.cycle_sort();
        assert_eq!(app.sort, ShelfSort::Recent);
    }

    #[test]
    fn move_selection_wraps_around() {
        let mut app = ShelfApp {
            books: vec![
                row("a", "A", None),
                row("b", "B", None),
                row("c", "C", None),
            ],
            visible: vec![0, 1, 2],
            selection: 0,
            sort: ShelfSort::Title,
            query: String::new(),
            mode: ShelfMode::Browse,
            viewport: Rect::new(0, 0, 80, 24),
            message: String::new(),
            pending_delete_file: None,
            filter: ShelfFilter::All,
            network_client: None,
            pending_network: None,
            pending_download: None,
            is_syncing_network: false,
        };
        app.move_selection(-1);
        assert_eq!(app.selection, 2);
        app.move_selection(1);
        assert_eq!(app.selection, 0);
    }

    #[test]
    fn enter_returns_open_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.txt");
        std::fs::write(&path, "hello").unwrap();
        let document = crate::book::BookDocument {
            id: "id".into(),
            title: "T".into(),
            path: path.clone(),
            format: crate::book::BookFormat::Txt,
            metadata: crate::book::BookMetadata {
                title: "T".into(),
                author: None,
                language: None,
                publisher: None,
                format: crate::book::BookFormat::Txt,
                path: path.clone(),
                section_count: 1,
            },
            sections: vec![crate::book::Section {
                title: "S".into(),
                text: "hello".into(),
            }],
            contents: vec![crate::book::ContentsEntry {
                title: "S".into(),
                depth: 0,
                target: crate::book::TextPosition::new(0, 0),
            }],
            contents_source: crate::book::ContentsSource::Generated,
        };
        let mut store = Store::open(dir.path().join("library.json")).unwrap();
        store.add_book(&document, &document.path).unwrap();
        let mut app = ShelfApp {
            books: vec![row("id", "T", None)],
            visible: vec![0],
            selection: 0,
            sort: ShelfSort::Title,
            query: String::new(),
            mode: ShelfMode::Browse,
            viewport: Rect::new(0, 0, 80, 24),
            message: String::new(),
            pending_delete_file: None,
            filter: ShelfFilter::All,
            network_client: None,
            pending_network: None,
            pending_download: None,
            is_syncing_network: false,
        };
        let outcome = handle_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut app,
            &mut store,
            &mut ReaderConfig::default(),
        )
        .unwrap();
        assert!(matches!(outcome, Some(ShelfOutcome::Open(_))));
    }

    #[test]
    fn r_with_no_recent_sets_message_and_does_not_exit() {
        let mut store = make_empty_store();
        let mut app = ShelfApp {
            books: vec![row("a", "A", None)],
            visible: vec![0],
            selection: 0,
            sort: ShelfSort::Title,
            query: String::new(),
            mode: ShelfMode::Browse,
            viewport: Rect::new(0, 0, 80, 24),
            message: String::new(),
            pending_delete_file: None,
            filter: ShelfFilter::All,
            network_client: None,
            pending_network: None,
            pending_download: None,
            is_syncing_network: false,
        };
        let outcome = handle_key(
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
            &mut app,
            &mut store,
            &mut ReaderConfig::default(),
        )
        .unwrap();
        assert!(outcome.is_none());
        assert!(app.message.contains("No recent"));
    }

    #[test]
    fn r_with_recent_returns_open_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "x").unwrap();
        let document = crate::book::BookDocument {
            id: "a".into(),
            title: "A".into(),
            path: path.clone(),
            format: crate::book::BookFormat::Txt,
            metadata: crate::book::BookMetadata {
                title: "A".into(),
                author: None,
                language: None,
                publisher: None,
                format: crate::book::BookFormat::Txt,
                path: path.clone(),
                section_count: 1,
            },
            sections: vec![crate::book::Section {
                title: "S".into(),
                text: "x".into(),
            }],
            contents: vec![crate::book::ContentsEntry {
                title: "S".into(),
                depth: 0,
                target: crate::book::TextPosition::new(0, 0),
            }],
            contents_source: crate::book::ContentsSource::Generated,
        };
        let mut store = Store::open(dir.path().join("library.json")).unwrap();
        store.add_book(&document, &document.path).unwrap();
        store.record_open("a");
        let mut app = ShelfApp {
            books: vec![row("a", "A", None)],
            visible: vec![0],
            selection: 0,
            sort: ShelfSort::Title,
            query: String::new(),
            mode: ShelfMode::Browse,
            viewport: Rect::new(0, 0, 80, 24),
            message: String::new(),
            pending_delete_file: None,
            filter: ShelfFilter::All,
            network_client: None,
            pending_network: None,
            pending_download: None,
            is_syncing_network: false,
        };
        let outcome = handle_key(
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
            &mut app,
            &mut store,
            &mut ReaderConfig::default(),
        )
        .unwrap();
        assert!(matches!(outcome, Some(ShelfOutcome::Open(_))));
    }

    #[test]
    fn q_exits_from_browse_mode() {
        let mut store = make_empty_store();
        let mut app = ShelfApp {
            books: vec![row("a", "A", None)],
            visible: vec![0],
            selection: 0,
            sort: ShelfSort::Title,
            query: String::new(),
            mode: ShelfMode::Browse,
            viewport: Rect::new(0, 0, 80, 24),
            message: String::new(),
            pending_delete_file: None,
            filter: ShelfFilter::All,
            network_client: None,
            pending_network: None,
            pending_download: None,
            is_syncing_network: false,
        };
        let outcome = handle_key(
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
            &mut app,
            &mut store,
            &mut ReaderConfig::default(),
        )
        .unwrap();
        assert!(matches!(outcome, Some(ShelfOutcome::Exit)));
    }

    #[test]
    fn search_enter_applies_filter() {
        let mut store = make_empty_store();
        let mut app = ShelfApp {
            books: vec![row("a", "Alpha", None), row("b", "Beta", None)],
            visible: vec![0, 1],
            selection: 0,
            sort: ShelfSort::Title,
            query: "alp".into(),
            mode: ShelfMode::Search,
            viewport: Rect::new(0, 0, 80, 24),
            message: String::new(),
            pending_delete_file: None,
            filter: ShelfFilter::All,
            network_client: None,
            pending_network: None,
            pending_download: None,
            is_syncing_network: false,
        };
        handle_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut app,
            &mut store,
            &mut ReaderConfig::default(),
        )
        .unwrap();
        assert_eq!(app.mode, ShelfMode::Browse);
        assert_eq!(app.visible, vec![0]);
    }

    fn make_empty_store() -> Store {
        let dir = tempfile::tempdir().unwrap();
        Store::open(dir.path().join("library.json")).unwrap()
    }

    #[test]
    fn search_esc_cancels_and_restores_full_list() {
        let mut store = make_empty_store();
        let mut app = ShelfApp {
            books: vec![row("a", "Alpha", None), row("b", "Beta", None)],
            visible: vec![0, 1],
            selection: 0,
            sort: ShelfSort::Title,
            query: "alp".into(),
            mode: ShelfMode::Search,
            viewport: Rect::new(0, 0, 80, 24),
            message: String::new(),
            pending_delete_file: None,
            filter: ShelfFilter::All,
            network_client: None,
            pending_network: None,
            pending_download: None,
            is_syncing_network: false,
        };
        handle_key(
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            &mut app,
            &mut store,
            &mut ReaderConfig::default(),
        )
        .unwrap();
        assert_eq!(app.mode, ShelfMode::Browse);
        assert_eq!(app.visible.len(), 2);
        assert!(app.query.is_empty());
    }

    #[test]
    fn delete_confirm_y_removes_book_from_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.txt");
        std::fs::write(&path, "x").unwrap();
        let document = crate::book::BookDocument {
            id: "abc".into(),
            title: "Book".into(),
            path: path.clone(),
            format: crate::book::BookFormat::Txt,
            metadata: crate::book::BookMetadata {
                title: "Book".into(),
                author: None,
                language: None,
                publisher: None,
                format: crate::book::BookFormat::Txt,
                path: path.clone(),
                section_count: 1,
            },
            sections: vec![crate::book::Section {
                title: "S".into(),
                text: "x".into(),
            }],
            contents: vec![crate::book::ContentsEntry {
                title: "S".into(),
                depth: 0,
                target: crate::book::TextPosition::new(0, 0),
            }],
            contents_source: crate::book::ContentsSource::Generated,
        };
        let mut store = Store::open(dir.path().join("library.json")).unwrap();
        store.add_book(&document, &document.path).unwrap();
        let mut app = ShelfApp {
            books: vec![row("abc", "Book", None)],
            visible: vec![0],
            selection: 0,
            sort: ShelfSort::Title,
            query: String::new(),
            mode: ShelfMode::ConfirmDelete,
            viewport: Rect::new(0, 0, 80, 24),
            message: "Delete \"Book\"? (y/n)".into(),
            pending_delete_file: None,
            filter: ShelfFilter::All,
            network_client: None,
            pending_network: None,
            pending_download: None,
            is_syncing_network: false,
        };
        let outcome = handle_key(
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
            &mut app,
            &mut store,
            &mut ReaderConfig::default(),
        )
        .unwrap();
        assert!(outcome.is_none());
        assert_eq!(app.mode, ShelfMode::Browse);
        assert!(app.message.contains("Removed: Book"));
        assert!(app.books.is_empty());
        assert_eq!(store.library().books().len(), 0);
    }

    #[test]
    fn delete_confirm_n_cancels() {
        let mut store = make_empty_store();
        let mut app = ShelfApp {
            books: vec![row("abc", "Book", None)],
            visible: vec![0],
            selection: 0,
            sort: ShelfSort::Title,
            query: String::new(),
            mode: ShelfMode::ConfirmDelete,
            viewport: Rect::new(0, 0, 80, 24),
            message: "Delete \"Book\"? (y/n)".into(),
            pending_delete_file: None,
            filter: ShelfFilter::All,
            network_client: None,
            pending_network: None,
            pending_download: None,
            is_syncing_network: false,
        };
        let outcome = handle_key(
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE),
            &mut app,
            &mut store,
            &mut ReaderConfig::default(),
        )
        .unwrap();
        assert!(outcome.is_none());
        assert_eq!(app.mode, ShelfMode::Browse);
        assert!(app.message.contains("cancelled"));
    }

    #[test]
    fn progress_bar_handles_full_and_zero() {
        assert_eq!(
            progress_bar(0, 5),
            "\u{2591}\u{2591}\u{2591}\u{2591}\u{2591}"
        );
        assert_eq!(
            progress_bar(100, 5),
            "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}"
        );
        assert_eq!(
            progress_bar(40, 5),
            "\u{2588}\u{2588}\u{2591}\u{2591}\u{2591}"
        );
    }

    #[test]
    fn shelf_columns_use_terminal_display_width() {
        for value in [
            "中文书名",
            "English title",
            "中文 mixed title",
            "Cafe\u{301}",
            "👨‍👩‍👧‍👦 family",
        ] {
            assert_eq!(fit_column(value, 12).width(), 12, "value: {value}");
            assert!(truncate(value, 6).width() <= 6, "value: {value}");
        }

        let row = row("cjk", "终端里的中文书名非常长", None);
        let line = build_row_line(&row, false, None, Palette::terminal());
        let text = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        let bar_start = text
            .find(['\u{2588}', '\u{2591}'])
            .expect("row should contain a progress bar");
        assert_eq!(text[..bar_start].width(), 54);
    }

    #[test]
    fn shelf_cycle_theme_updates_config_and_message() {
        let mut store = make_empty_store();
        let mut app = ShelfApp {
            books: vec![],
            visible: vec![],
            selection: 0,
            sort: ShelfSort::Title,
            query: String::new(),
            mode: ShelfMode::Browse,
            viewport: Rect::new(0, 0, 80, 24),
            message: String::new(),
            pending_delete_file: None,
            filter: ShelfFilter::All,
            network_client: None,
            pending_network: None,
            pending_download: None,
            is_syncing_network: false,
        };
        let mut config = ReaderConfig::default();
        assert_eq!(config.reading_theme, crate::ui::ReadingTheme::Terminal);

        let outcome = handle_key(
            KeyEvent::new(KeyCode::Char('C'), KeyModifiers::NONE),
            &mut app,
            &mut store,
            &mut config,
        )
        .unwrap();

        assert!(outcome.is_none());
        assert_eq!(config.reading_theme, crate::ui::ReadingTheme::Sepia);
        assert!(app.message.contains("Warm Sepia"));
    }

    #[test]
    fn format_relative_time_caps_at_date() {
        let now = SystemTime::now();
        assert_eq!(
            format_relative_time(now - Duration::from_secs(5)),
            "just now"
        );
        assert_eq!(format_relative_time(now - Duration::from_secs(120)), "2m");
        assert_eq!(format_relative_time(now - Duration::from_secs(7200)), "2h");
        assert_eq!(
            format_relative_time(now - Duration::from_secs(86_400 * 3)),
            "3d"
        );
    }

    #[test]
    fn shelf_inherits_terminal_background_and_renders_help() {
        let store = make_empty_store();
        let mut app = ShelfApp {
            books: vec![row("a", "Alpha", None)],
            visible: vec![0],
            selection: 0,
            sort: ShelfSort::Title,
            query: String::new(),
            mode: ShelfMode::Help,
            viewport: Rect::new(0, 0, 80, 20),
            message: String::new(),
            pending_delete_file: None,
            filter: ShelfFilter::All,
            network_client: None,
            pending_network: None,
            pending_download: None,
            is_syncing_network: false,
        };
        app.books[0].progress_tenths = 500;
        let config = ReaderConfig::default();
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| draw(frame, &app, &store, &config))
            .unwrap();

        let cell = terminal.backend().buffer().cell((0, 0)).unwrap();
        assert_eq!(cell.bg, ratatui::style::Color::Reset);
        let screen = format!("{}", terminal.backend());
        assert!(screen.contains("Shelf help"));
        assert!(screen.contains("Open selected book"));
    }

    #[test]
    fn sort_recent_puts_read_local_before_unread_network() {
        let now = SystemTime::now();
        let mut app = ShelfApp {
            books: vec![
                ShelfRow {
                    id: "net1".into(),
                    title: "Zebra Network Book".into(),
                    author: None,
                    format: BookFormat::Epub,
                    progress_tenths: 0,
                    last_opened: None,
                    bookmark_count: 0,
                    missing: false,
                    path: PathBuf::new(),
                    source: BookSource::Remote(crate::network::RemoteBook {
                        id: "net1".into(),
                        title: "Zebra Network Book".into(),
                        author: None,
                        format: Some(BookFormat::Epub),
                        download_url: Some(
                            reqwest::Url::parse("https://example.com/b.epub").unwrap(),
                        ),
                    }),
                },
                ShelfRow {
                    id: "local_read".into(),
                    title: "Alpha Read Local".into(),
                    author: None,
                    format: BookFormat::Txt,
                    progress_tenths: 100,
                    last_opened: Some(now),
                    bookmark_count: 0,
                    missing: false,
                    path: PathBuf::from("/tmp/a.txt"),
                    source: BookSource::Local,
                },
                ShelfRow {
                    id: "net2".into(),
                    title: "Apple Network Book".into(),
                    author: None,
                    format: BookFormat::Epub,
                    progress_tenths: 0,
                    last_opened: None,
                    bookmark_count: 0,
                    missing: false,
                    path: PathBuf::new(),
                    source: BookSource::Remote(crate::network::RemoteBook {
                        id: "net2".into(),
                        title: "Apple Network Book".into(),
                        author: None,
                        format: Some(BookFormat::Epub),
                        download_url: Some(
                            reqwest::Url::parse("https://example.com/a.epub").unwrap(),
                        ),
                    }),
                },
            ],
            visible: vec![],
            selection: 0,
            sort: ShelfSort::Recent,
            query: String::new(),
            mode: ShelfMode::Browse,
            viewport: Rect::new(0, 0, 80, 24),
            message: String::new(),
            pending_delete_file: None,
            filter: ShelfFilter::All,
            network_client: None,
            pending_network: None,
            pending_download: None,
            is_syncing_network: false,
        };
        app.recompute_visible();
        let ordered_ids: Vec<&str> = app
            .visible
            .iter()
            .map(|&i| app.books[i].id.as_str())
            .collect();
        assert_eq!(ordered_ids, vec!["local_read", "net2", "net1"]);
    }

    #[test]
    fn merge_remote_books_deduplicates_with_local() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.txt");
        std::fs::write(&path, "content").unwrap();
        let document = crate::book::BookDocument {
            id: "local_id".into(),
            title: "Same Title".into(),
            path: path.clone(),
            format: crate::book::BookFormat::Txt,
            metadata: crate::book::BookMetadata {
                title: "Same Title".into(),
                author: Some("Author A".into()),
                language: None,
                publisher: None,
                format: crate::book::BookFormat::Txt,
                path: path.clone(),
                section_count: 1,
            },
            sections: vec![crate::book::Section {
                title: "S".into(),
                text: "content".into(),
            }],
            contents: vec![],
            contents_source: crate::book::ContentsSource::Generated,
        };
        let mut store = Store::open(dir.path().join("library.json")).unwrap();
        store.add_book(&document, &document.path).unwrap();

        let mut app = ShelfApp::new(&store, Rect::new(0, 0, 80, 24), None);
        assert_eq!(app.books.len(), 1);

        let remotes = vec![
            crate::network::RemoteBook {
                id: "remote_dup".into(),
                title: "Same Title".into(),
                author: Some("Author A".into()),
                format: Some(BookFormat::Epub),
                download_url: Some(reqwest::Url::parse("https://example.com/1.epub").unwrap()),
            },
            crate::network::RemoteBook {
                id: "remote_new".into(),
                title: "Different Title".into(),
                author: Some("Author B".into()),
                format: Some(BookFormat::Epub),
                download_url: Some(reqwest::Url::parse("https://example.com/2.epub").unwrap()),
            },
        ];

        app.merge_remote_books(remotes, &store);
        // Only 1 duplicate skipped, 1 new added => total 2 books
        assert_eq!(app.books.len(), 2);
        assert_eq!(app.books[0].id, "local_id");
        assert_eq!(app.books[1].id, "remote_new");
    }

    #[test]
    fn cycle_filter_cycles_through_all_local_remote() {
        let mut app = ShelfApp {
            books: vec![
                ShelfRow {
                    id: "local".into(),
                    title: "Local Book".into(),
                    author: None,
                    format: BookFormat::Txt,
                    progress_tenths: 0,
                    last_opened: None,
                    bookmark_count: 0,
                    missing: false,
                    path: PathBuf::new(),
                    source: BookSource::Local,
                },
                ShelfRow {
                    id: "remote".into(),
                    title: "Remote Book".into(),
                    author: None,
                    format: BookFormat::Epub,
                    progress_tenths: 0,
                    last_opened: None,
                    bookmark_count: 0,
                    missing: false,
                    path: PathBuf::new(),
                    source: BookSource::Remote(crate::network::RemoteBook {
                        id: "remote".into(),
                        title: "Remote Book".into(),
                        author: None,
                        format: Some(BookFormat::Epub),
                        download_url: Some(
                            reqwest::Url::parse("https://example.com/r.epub").unwrap(),
                        ),
                    }),
                },
            ],
            visible: vec![],
            selection: 0,
            sort: ShelfSort::Title,
            query: String::new(),
            mode: ShelfMode::Browse,
            viewport: Rect::new(0, 0, 80, 24),
            message: String::new(),
            pending_delete_file: None,
            filter: ShelfFilter::All,
            network_client: None,
            pending_network: None,
            pending_download: None,
            is_syncing_network: false,
        };
        app.recompute_visible();
        assert_eq!(app.visible.len(), 2);

        app.cycle_filter();
        assert_eq!(app.filter, ShelfFilter::Local);
        assert_eq!(app.visible.len(), 1);
        assert_eq!(app.books[app.visible[0]].id, "local");

        app.cycle_filter();
        assert_eq!(app.filter, ShelfFilter::Remote);
        assert_eq!(app.visible.len(), 1);
        assert_eq!(app.books[app.visible[0]].id, "remote");

        app.cycle_filter();
        assert_eq!(app.filter, ShelfFilter::All);
        assert_eq!(app.visible.len(), 2);
    }
}
