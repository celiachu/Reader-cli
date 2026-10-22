use std::sync::mpsc::{self, Receiver};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use reqwest::Url;

use crate::config::ReaderConfig;
use crate::network::{CatalogPage, DownloadedBook, NetworkClient, RemoteBook};
use crate::shelf::ShelfOutcome;
use crate::storage::Store;
use crate::ui::{
    action_line, centered_rect, command_for, fill_background, help_lines, hud_chunks, KeyBinding,
    KeyChord, Palette,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Open,
    Next,
    Previous,
    Search,
    NextPage,
    PreviousPage,
    Refresh,
    Local,
    Help,
    Quit,
}

macro_rules! binding {
    ($action:ident, $code:expr, $keys:literal, $label:literal, $short:literal, $priority:expr) => {
        KeyBinding {
            command: Action::$action,
            chords: &[KeyChord::plain($code)],
            keys: $keys,
            hud_keys: $keys,
            label: $label,
            short_label: $short,
            show_in_hud: true,
            hud_priority: $priority,
        }
    };
}

const BINDINGS: &[KeyBinding<Action>] = &[
    binding!(
        Open,
        KeyCode::Enter,
        "Enter",
        "Download and read",
        "Read",
        0
    ),
    KeyBinding {
        command: Action::Next,
        chords: &[
            KeyChord::plain(KeyCode::Char('j')),
            KeyChord::plain(KeyCode::Down),
        ],
        keys: "j / Down",
        hud_keys: "Down",
        label: "Next book",
        short_label: "Next",
        show_in_hud: true,
        hud_priority: 1,
    },
    KeyBinding {
        command: Action::Previous,
        chords: &[
            KeyChord::plain(KeyCode::Char('k')),
            KeyChord::plain(KeyCode::Up),
        ],
        keys: "k / Up",
        hud_keys: "Up",
        label: "Previous book",
        short_label: "Prev",
        show_in_hud: true,
        hud_priority: 1,
    },
    binding!(
        Search,
        KeyCode::Char('/'),
        "/",
        "Search catalog",
        "Search",
        0
    ),
    binding!(
        NextPage,
        KeyCode::Char('n'),
        "n",
        "Next catalog page",
        "Next page",
        1
    ),
    binding!(
        PreviousPage,
        KeyCode::Char('p'),
        "p",
        "Previous catalog page",
        "Prev page",
        1
    ),
    binding!(
        Refresh,
        KeyCode::Char('R'),
        "R",
        "Reload current catalog page",
        "Refresh",
        1
    ),
    binding!(
        Local,
        KeyCode::Char('w'),
        "w",
        "Local bookshelf",
        "Local",
        0
    ),
    KeyBinding {
        command: Action::Help,
        chords: &[
            KeyChord::plain(KeyCode::Char('?')),
            KeyChord::plain(KeyCode::F(1)),
        ],
        keys: "? / F1",
        hud_keys: "?",
        label: "Full help",
        short_label: "Help",
        show_in_hud: true,
        hud_priority: 0,
    },
    KeyBinding {
        command: Action::Quit,
        chords: &[
            KeyChord::plain(KeyCode::Char('q')),
            KeyChord::plain(KeyCode::Esc),
            KeyChord::control('c'),
        ],
        keys: "q / Esc / Ctrl+C",
        hud_keys: "q",
        label: "Quit",
        short_label: "Quit",
        show_in_hud: true,
        hud_priority: 0,
    },
];

enum Work {
    Page(CatalogPage),
    Download(DownloadedBook),
}

struct NetworkShelf {
    books: Vec<RemoteBook>,
    next: Option<Url>,
    history: Vec<Url>,
    page_index: usize,
    selection: usize,
    query: String,
    search_input: Option<String>,
    help: bool,
    message: String,
    pending: Option<Receiver<Result<Work>>>,
}

impl NetworkShelf {
    fn new(url: Url) -> Self {
        Self {
            books: Vec::new(),
            next: None,
            history: vec![url],
            page_index: 0,
            selection: 0,
            query: String::new(),
            search_input: None,
            help: false,
            message: String::new(),
            pending: None,
        }
    }

    fn load(&mut self, client: &NetworkClient) {
        let url = self.history[self.page_index].clone();
        let client = client.clone();
        self.start("Loading network books...", move || {
            client.page(url).map(Work::Page)
        });
    }

    fn advance_page(&mut self) -> bool {
        let Some(url) = self.next.clone() else {
            return false;
        };
        if self.history[..=self.page_index]
            .iter()
            .any(|visited| visited == &url)
        {
            self.message = "OPDS pagination loop detected".into();
            return false;
        }
        self.history.truncate(self.page_index + 1);
        self.history.push(url);
        self.page_index += 1;
        self.books.clear();
        true
    }

    fn start(&mut self, message: &str, operation: impl FnOnce() -> Result<Work> + Send + 'static) {
        let (sender, receiver) = mpsc::channel();
        self.pending = Some(receiver);
        self.message = message.to_string();
        std::thread::spawn(move || {
            let _ = sender.send(operation());
        });
    }

    fn poll(&mut self, store: &mut Store, config: &ReaderConfig) -> Option<ShelfOutcome> {
        let result = match self.pending.as_ref()?.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.pending = None;
                self.message =
                    "Network operation stopped. Press R to retry or w for local books.".into();
                return None;
            }
        };
        self.pending = None;
        match result {
            Ok(Work::Page(page)) => {
                self.books = page.books;
                self.next = page.next;
                self.selection = 0;
                self.message = if self.books.is_empty() {
                    "No network books found".into()
                } else {
                    "Enter downloads a local copy for reading".into()
                };
            }
            Ok(Work::Download(download)) => {
                match crate::network::import_download(store, config, &download) {
                    Ok(book) => return Some(ShelfOutcome::Open(book)),
                    Err(err) => self.message = format!("Import failed: {err:#}"),
                }
            }
            Err(err) => {
                // Do not keep a previous page displayed under a failed page/search label.
                self.books.clear();
                self.next = None;
                self.message = format!("{err:#}  [R] Retry  [w] Local");
            }
        }
        None
    }

    fn action(
        &mut self,
        action: Action,
        client: &NetworkClient,
        config: &ReaderConfig,
    ) -> Option<ShelfOutcome> {
        match action {
            Action::Quit => return Some(ShelfOutcome::Exit),
            Action::Local => return Some(ShelfOutcome::Local),
            Action::Help => {
                self.help = true;
                return None;
            }
            _ if self.pending.is_some() => return None,
            Action::Next if !self.books.is_empty() => {
                self.selection = (self.selection + 1) % self.books.len()
            }
            Action::Previous if !self.books.is_empty() => {
                self.selection = (self.selection + self.books.len() - 1) % self.books.len()
            }
            Action::Search => self.search_input = Some(self.query.clone()),
            Action::NextPage if self.advance_page() => {
                self.load(client);
            }
            Action::NextPage => {}
            Action::PreviousPage if self.page_index > 0 => {
                self.page_index -= 1;
                self.books.clear();
                self.load(client);
            }
            Action::Refresh => self.load(client),
            Action::Open => {
                if config.bookshelf_dir.is_none() {
                    self.message =
                        "Set a local download folder with `reader refresh <DIR>` first. [w] Local"
                            .into();
                } else if let Some(book) = self.books.get(self.selection).cloned() {
                    if book.format.is_none() {
                        self.message =
                            "This book has no EPUB, TXT or PDF. Convert it in Calibre first."
                                .into();
                    } else {
                        let client = client.clone();
                        self.start("Downloading book... [w] Local  [q] Quit", move || {
                            client.download(&book).map(Work::Download)
                        });
                    }
                }
            }
            _ => {}
        }
        None
    }
}

struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

pub fn run_network_shelf(
    store: &mut Store,
    config: &ReaderConfig,
    client: &NetworkClient,
) -> Result<ShelfOutcome> {
    let mut app = NetworkShelf::new(client.catalog_url("")?);
    app.load(client);
    let mut terminal = ratatui::init();
    let _guard = TerminalGuard;
    loop {
        if let Some(outcome) = app.poll(store, config) {
            return Ok(outcome);
        }
        terminal.draw(|frame| draw(frame, &app))?;
        if !event::poll(std::time::Duration::from_millis(100))? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if key.code == KeyCode::Char('c')
                && key.modifiers.contains(event::KeyModifiers::CONTROL)
            {
                return Ok(ShelfOutcome::Exit);
            }
            if app.help {
                if matches!(
                    key.code,
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Char('?') | KeyCode::F(1)
                ) {
                    app.help = false;
                }
                continue;
            }
            if let Some(input) = app.search_input.as_mut() {
                match key.code {
                    KeyCode::Esc => app.search_input = None,
                    KeyCode::Backspace => {
                        input.pop();
                    }
                    KeyCode::Char(ch) => input.push(ch),
                    KeyCode::Enter => {
                        let query = app.search_input.take().unwrap_or_default();
                        app.history = vec![client.catalog_url(&query)?];
                        app.page_index = 0;
                        app.query = query;
                        app.books.clear();
                        app.load(client);
                    }
                    _ => {}
                }
                continue;
            }
            if let Some(action) = command_for(BINDINGS, key) {
                if let Some(outcome) = app.action(action, client, config) {
                    return Ok(outcome);
                }
            }
        }
    }
}

fn draw(frame: &mut ratatui::Frame<'_>, app: &NetworkShelf) {
    let palette = Palette::terminal();
    fill_background(frame, palette);
    let [status_area, body_area, action_area] = hud_chunks(frame.area());
    let status = if let Some(input) = &app.search_input {
        format!("Network search: /{input}")
    } else {
        format!(
            "Network Library [ON]  ·  page {}  ·  {} books{}{}",
            app.page_index + 1,
            app.books.len(),
            if app.next.is_some() {
                "  ·  more: n"
            } else {
                ""
            },
            if app.query.is_empty() {
                String::new()
            } else {
                format!("  ·  search: {}", app.query)
            }
        )
    };
    frame.render_widget(Paragraph::new(status).style(palette.accent()), status_area);
    let list_area = Rect {
        height: body_area.height.saturating_sub(3),
        ..body_area
    };
    let scroll = app
        .selection
        .saturating_sub((list_area.height as usize).saturating_sub(1));
    let lines: Vec<_> = app
        .books
        .iter()
        .enumerate()
        .skip(scroll)
        .take(list_area.height as usize)
        .map(|(index, book)| {
            let marker = if index == app.selection { ">" } else { " " };
            let format = book
                .format
                .map(|format| format.label())
                .unwrap_or("Unsupported");
            Line::from(Span::styled(
                format!(
                    "{marker} [{format}] {}  ·  {}",
                    book.title,
                    book.author.as_deref().unwrap_or("-")
                ),
                if index == app.selection {
                    palette.selected()
                } else {
                    palette.body()
                },
            ))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), list_area);
    let message_area = Rect::new(
        body_area.x,
        list_area.bottom(),
        body_area.width,
        body_area.height.saturating_sub(list_area.height),
    );
    frame.render_widget(
        Paragraph::new(app.message.clone())
            .style(palette.muted())
            .wrap(Wrap { trim: false }),
        message_area,
    );
    let actions = if app.search_input.is_some() {
        Line::from("[Enter] Search  [Esc] Cancel  [Backspace] Edit")
    } else {
        action_line(BINDINGS, frame.area().width, palette)
    };
    frame.render_widget(Paragraph::new(actions).style(palette.base()), action_area);
    if app.help {
        let area = centered_rect(80, 85, frame.area());
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(help_lines("Network shelf", BINDINGS, palette))
                .block(
                    Block::default()
                        .title(" Network shelf help ")
                        .borders(Borders::ALL)
                        .style(palette.popup()),
                )
                .style(palette.popup())
                .wrap(Wrap { trim: false }),
            area,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn can_move_forward_again_after_returning_to_previous_page() {
        let first = Url::parse("https://books.example/opds/books/letter/00").unwrap();
        let second = first.join("?offset=20").unwrap();
        let mut app = NetworkShelf::new(first.clone());
        app.next = Some(second.clone());
        assert!(app.advance_page());
        assert_eq!(app.page_index, 1);
        app.page_index -= 1;
        app.next = Some(second.clone());
        assert!(app.advance_page());
        assert_eq!(app.history, [first.clone(), second]);
        app.next = Some(first);
        assert!(!app.advance_page());
        assert!(app.message.contains("loop"));
    }

    #[test]
    fn local_switch_works_while_a_request_is_pending_and_keeps_library_data() {
        let config = ReaderConfig::default();
        let client = NetworkClient::new(&crate::config::NetworkLibraryConfig {
            url: Some("https://books.example".into()),
            ..Default::default()
        })
        .unwrap();
        let mut app = NetworkShelf::new(client.catalog_url("").unwrap());
        let (_sender, receiver) = mpsc::channel();
        app.pending = Some(receiver);
        assert!(matches!(
            app.action(Action::Local, &client, &config),
            Some(ShelfOutcome::Local)
        ));
        assert!(matches!(
            app.action(Action::Quit, &client, &config),
            Some(ShelfOutcome::Exit)
        ));
        assert!(app.action(Action::Open, &client, &config).is_none());
    }

    #[test]
    fn failed_network_page_clears_stale_results_and_keeps_local_store() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("local.txt");
        std::fs::write(&file, "本地书籍").unwrap();
        let document = crate::book::load_book(&file).unwrap();
        let mut store = Store::open(dir.path().join("library.json")).unwrap();
        store.add_book(&document, &file).unwrap();
        let mut app = NetworkShelf::new(Url::parse("https://books.example/opds").unwrap());
        app.books.push(RemoteBook {
            id: "stale".into(),
            title: "Old page".into(),
            author: None,
            format: None,
            download_url: None,
        });
        let (sender, receiver) = mpsc::channel();
        sender.send(Err(anyhow::anyhow!("offline"))).unwrap();
        app.pending = Some(receiver);
        assert!(app.poll(&mut store, &ReaderConfig::default()).is_none());
        assert!(app.books.is_empty());
        assert!(app.message.contains("offline"));
        assert_eq!(store.library().books().len(), 1);
        assert_eq!(std::fs::read_to_string(file).unwrap(), "本地书籍");
    }

    #[test]
    fn network_status_and_toggle_render_with_terminal_colors() {
        let app = NetworkShelf::new(Url::parse("https://books.example/opds").unwrap());
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("Network Library [ON]"));
        assert!(text.contains("Local"));
        assert_eq!(
            command_for(
                BINDINGS,
                event::KeyEvent::new(KeyCode::Char('w'), event::KeyModifiers::NONE)
            ),
            Some(Action::Local)
        );
    }
}
