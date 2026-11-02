use std::time::{Duration, Instant};

use ratatui::layout::Rect;

use crate::book::{BookDocument, ContentMetrics, SearchResult, TextPosition};
use crate::config::ReaderConfig;
use crate::discreet::{AgentDensity, AgentLayout, AgentSession, ReadDirection};
use crate::pagination::{page_for_position, paginate_with_options, Page};
use crate::storage::{Bookmark, Progress};

const NOTICE_DURATION: Duration = Duration::from_millis(2800);

#[derive(Debug)]
pub(crate) struct ReaderState {
    pub document: BookDocument,
    pub book_id: String,
    pub metrics: ContentMetrics,
    pub pages: Vec<Page>,
    pub current_page: usize,
    pub position: TextPosition,
    pub viewport: Rect,
    pub overlay: Overlay,
    pub notification: Option<Notification>,
    pub search_results: Vec<SearchResult>,
    pub search_query: Option<String>,
    pub search_index: usize,
    pub bookmarks: Vec<Bookmark>,
    pub config: ReaderConfig,
    pub hidden: bool,
    pub agent: AgentSession,
    pub auto_reading: crate::auto_reading::AutoReadingState,
    pub line_scroll_offset: usize,
}

#[derive(Debug, Default)]
pub(crate) enum Overlay {
    #[default]
    None,
    Percent(PercentNavigator),
    Contents(ContentsNavigator),
    Search(SearchInput),
    SearchResults(SearchResultsNavigator),
    Bookmarks(BookmarksNavigator),
    Metadata,
    Help,
}

#[derive(Debug, Default)]
pub(crate) struct PercentNavigator {
    pub input: String,
    pub error: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct ContentsNavigator {
    pub selection: usize,
    pub number_input: String,
    pub filter_active: bool,
    pub filter_query: String,
    pub error: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct SearchInput {
    pub query: String,
    pub error: Option<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct SearchResultsNavigator {
    pub selection: usize,
    pub error: Option<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct BookmarksNavigator {
    pub selection: usize,
    pub error: Option<String>,
}

#[derive(Debug)]
pub(crate) struct Notification {
    pub text: String,
    pub kind: NotificationKind,
    expires_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NotificationKind {
    Info,
    Success,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReaderAction {
    None,
    Exit,
    NextScreen,
    PreviousScreen,
    JumpStart,
    JumpEnd,
    OpenPercent,
    OpenContents,
    OpenSearch,
    OpenSearchResults,
    OpenBookmarks,
    OpenMetadata,
    OpenHelp,
    ToggleProgress,
    ToggleDiscreet,
    ToggleHidden,
    ToggleImmersive,
    TogglePageOverlap,
    AdjustTextWidth(i16),
    ResetTextWidth,
    AddBookmark,
    DeleteBookmark,
    NextSearch,
    PreviousSearch,
    Move(isize),
    Confirm,
    Cancel,
    Backspace,
    StartFilter,
    Input(char),
    DiscreetUnavailable,
    ScrollLines(isize),
    OpenCommand,
    CompleteCommand,
    ToggleDensity,
    SetAgentLines(Option<u16>),
    CycleTheme,
    #[allow(dead_code)]
    SetReadingTheme(crate::ui::ReadingTheme),
    ToggleAutoReading,
    ToggleAutoReadingPause,
    StopAutoReading,
    AdjustAutoReadingSpeed(i32),
    SetAutoReadingSpeed(u32),
    SetAutoReadingMode(crate::auto_reading::AutoReadingMode),
    CalibrateReadingSpeed,
    AutoTick,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReaderEffect {
    SaveProgress(Progress),
    SaveConfig,
    AddBookmark {
        label: String,
        progress: Progress,
    },
    DeleteBookmark {
        id: String,
        removed: Bookmark,
        index: usize,
    },
    Exit,
}

impl ReaderState {
    pub fn new(
        document: BookDocument,
        book_id: String,
        viewport: Rect,
        config: ReaderConfig,
        saved: Progress,
        bookmarks: Vec<Bookmark>,
    ) -> Self {
        let metrics = document.content_metrics();
        let position = metrics.clamp(saved.position);
        let pages = paginate_for_config(&document, viewport, &config);
        let current_page = page_for_position(&pages, position);
        let auto_reading = crate::auto_reading::AutoReadingState::new(
            config.auto_reading_cpm(),
            config.auto_reading_mode(),
        );
        let mut state = Self {
            document,
            book_id,
            metrics,
            pages,
            current_page,
            position,
            viewport,
            overlay: Overlay::None,
            notification: None,
            search_results: Vec::new(),
            search_query: None,
            search_index: 0,
            bookmarks,
            config,
            hidden: false,
            agent: AgentSession::default(),
            auto_reading,
            line_scroll_offset: 0,
        };
        state.reflow_agent();
        let chars = state.current_page_effective_chars();
        state
            .auto_reading
            .calibration
            .on_page_entered(chars, Instant::now());
        if state.config.discreet_mode {
            state.agent.record_read(ReadDirection::Current);
        }
        state
    }

    pub fn current_progress(&self) -> Progress {
        Progress::new(self.position)
    }

    pub fn progress_tenths(&self) -> u16 {
        self.metrics.progress_tenths(self.position)
    }

    pub fn current_section(&self) -> usize {
        if self.config.discreet_mode {
            return self.position.section_index;
        }
        self.pages
            .get(self.current_page)
            .map(|page| page.section_index)
            .unwrap_or(self.position.section_index)
            .min(self.document.sections.len().saturating_sub(1))
    }

    pub(crate) fn physical_row_range(&self) -> (usize, usize, usize) {
        if self.config.discreet_mode {
            let view = &self.agent.view;
            return (
                view.top + 1,
                (view.top + self.agent_body_height())
                    .min(view.lines.len())
                    .max(1),
                view.lines.len().max(1),
            );
        }
        let total = self
            .pages
            .iter()
            .map(|page| page.lines.len())
            .sum::<usize>()
            .max(1);
        let start = self
            .pages
            .iter()
            .take(self.current_page)
            .map(|page| page.lines.len())
            .sum::<usize>()
            .saturating_add(1)
            .min(total);
        let visible = self
            .pages
            .get(self.current_page)
            .map(|page| page.lines.len())
            .unwrap_or(1)
            .max(1);
        let end = start.saturating_add(visible.saturating_sub(1)).min(total);
        (start, end, total)
    }

    pub fn resize(&mut self, width: u16, height: u16) {
        self.viewport = Rect::new(0, 0, width, height);
        if !self.hidden {
            self.repaginate();
        }
    }

    pub fn expire_notification(&mut self, now: Instant) {
        if self
            .notification
            .as_ref()
            .is_some_and(|notification| notification.expires_at <= now)
        {
            self.notification = None;
        }
    }

    pub fn notify(&mut self, text: impl Into<String>, kind: NotificationKind) {
        self.notification = Some(Notification {
            text: text.into(),
            kind,
            expires_at: Instant::now() + NOTICE_DURATION,
        });
    }

    pub fn update(&mut self, action: ReaderAction) -> Vec<ReaderEffect> {
        if self.hidden && !matches!(action, ReaderAction::Exit | ReaderAction::ToggleHidden) {
            return Vec::new();
        }
        if self.config.discreet_mode {
            if self.agent.command.is_some()
                && matches!(
                    action,
                    ReaderAction::Input(_)
                        | ReaderAction::Backspace
                        | ReaderAction::Confirm
                        | ReaderAction::Cancel
                        | ReaderAction::CompleteCommand
                )
            {
                return self.update_command(action);
            }
            if self.agent.help
                && matches!(
                    action,
                    ReaderAction::Confirm | ReaderAction::Cancel | ReaderAction::OpenHelp
                )
            {
                self.agent.help = false;
                return Vec::new();
            }
        }
        if self.config.discreet_mode && !action_is_available_in_discreet(action) {
            self.notify("Unavailable in discreet mode", NotificationKind::Info);
            return Vec::new();
        }

        match action {
            ReaderAction::None => Vec::new(),
            ReaderAction::ScrollLines(delta) => {
                let effects = if self.config.discreet_mode {
                    self.scroll_agent(delta, false)
                } else {
                    self.scroll_lines_standard(delta)
                };
                if self.auto_reading.active {
                    let now = Instant::now();
                    self.reset_auto_reading_for_current_page(now);
                }
                effects
            }
            ReaderAction::OpenCommand => {
                self.agent.command = Some(String::new());
                Vec::new()
            }
            ReaderAction::CompleteCommand => Vec::new(),
            ReaderAction::ToggleDensity => {
                if self.config.discreet_mode {
                    self.agent.density = self.agent.density.toggle();
                    self.reflow_agent();
                    self.agent.receipt = Some(
                        match self.agent.density {
                            AgentDensity::Comfortable => "Density: comfortable",
                            AgentDensity::Inspect => "Density: inspect",
                        }
                        .into(),
                    );
                }
                Vec::new()
            }
            ReaderAction::SetAgentLines(lines) => {
                if self.config.discreet_mode {
                    self.config.agent_lines = lines;
                    self.reflow_agent();
                    self.agent.receipt = Some(
                        match lines {
                            Some(_) => "Agent rows updated",
                            None => "Agent rows: auto",
                        }
                        .into(),
                    );
                    vec![ReaderEffect::SaveConfig]
                } else {
                    Vec::new()
                }
            }
            ReaderAction::Exit => vec![ReaderEffect::Exit],
            ReaderAction::NextScreen => {
                let now = Instant::now();
                self.record_manual_page_turn(now);
                let effects = self.move_screen(1);
                if self.auto_reading.active {
                    self.reset_auto_reading_for_current_page(now);
                }
                effects
            }
            ReaderAction::PreviousScreen => {
                let now = Instant::now();
                self.record_manual_page_turn(now);
                let effects = self.move_screen(-1);
                if self.auto_reading.active {
                    self.reset_auto_reading_for_current_page(now);
                }
                effects
            }
            ReaderAction::ToggleAutoReading => {
                let now = Instant::now();
                if !self.auto_reading.active {
                    let chars = self.current_page_effective_chars();
                    let is_chapter = self.is_current_page_chapter_start();
                    let first_line = self.current_line_text();
                    self.auto_reading
                        .start(chars, is_chapter, first_line.as_deref(), now);
                    let cpm = self.auto_reading.cpm;
                    let mode_str = self.auto_reading.mode.label();
                    if self.config.discreet_mode {
                        self.agent.receipt = Some(match self.auto_reading.mode {
                            crate::auto_reading::AutoReadingMode::TimedPage => {
                                format!("Stream pace: {cpm} CPM ({mode_str})").into()
                            }
                            crate::auto_reading::AutoReadingMode::LineScroll => {
                                format!("Stream feed: {cpm} CPM ({mode_str})").into()
                            }
                        });
                    } else {
                        self.notify(
                            format!("Auto reading on: {cpm} CPM ({mode_str})"),
                            NotificationKind::Success,
                        );
                    }
                    Vec::new()
                } else {
                    self.update(ReaderAction::ToggleAutoReadingPause)
                }
            }
            ReaderAction::ToggleAutoReadingPause => {
                let now = Instant::now();
                if !self.auto_reading.active {
                    return self.update(ReaderAction::ToggleAutoReading);
                }
                let paused = self.auto_reading.toggle_pause(now);
                let cpm = self.auto_reading.cpm;
                if self.config.discreet_mode {
                    self.agent.receipt = Some(if paused {
                        format!("Stream pace: paused ({cpm} CPM)").into()
                    } else {
                        format!("Stream pace: {cpm} CPM").into()
                    });
                } else {
                    self.notify(
                        if paused {
                            format!("Auto reading paused · {cpm} CPM (Press a to resume)")
                        } else {
                            format!("Auto reading resumed · {cpm} CPM")
                        },
                        NotificationKind::Info,
                    );
                }
                Vec::new()
            }
            ReaderAction::StopAutoReading => {
                if self.auto_reading.active {
                    self.auto_reading.stop();
                    if self.config.discreet_mode {
                        self.agent.receipt = Some("Stream pace: off".into());
                    } else {
                        self.notify("Auto reading stopped", NotificationKind::Info);
                    }
                }
                Vec::new()
            }
            ReaderAction::AdjustAutoReadingSpeed(delta) => {
                let now = Instant::now();
                let chars = self.current_page_effective_chars();
                let is_chapter = self.is_current_page_chapter_start();
                let new_cpm = self.auto_reading.adjust_cpm(delta, chars, is_chapter, now);
                self.config.set_auto_reading_cpm(new_cpm);
                if self.config.discreet_mode {
                    self.agent.receipt = Some(format!("Stream pace: {new_cpm} CPM").into());
                } else {
                    self.notify(
                        format!("Reading speed: {new_cpm} CPM"),
                        NotificationKind::Info,
                    );
                }
                vec![ReaderEffect::SaveConfig]
            }
            ReaderAction::SetAutoReadingSpeed(cpm) => {
                let now = Instant::now();
                let chars = self.current_page_effective_chars();
                let is_chapter = self.is_current_page_chapter_start();
                self.auto_reading.set_cpm(cpm, chars, is_chapter, now);
                self.config.set_auto_reading_cpm(cpm);
                if self.config.discreet_mode {
                    self.agent.receipt = Some(format!("Stream pace: {cpm} CPM").into());
                } else {
                    self.notify(
                        format!("Reading speed set: {cpm} CPM"),
                        NotificationKind::Info,
                    );
                }
                vec![ReaderEffect::SaveConfig]
            }
            ReaderAction::SetAutoReadingMode(mode) => {
                let now = Instant::now();
                self.auto_reading.mode = mode;
                self.config.set_auto_reading_mode(mode);
                let chars = self.current_page_effective_chars();
                let is_chapter = self.is_current_page_chapter_start();
                self.auto_reading.reset_for_page(chars, is_chapter, now);
                if let Some(line) = self.current_line_text() {
                    self.auto_reading.reset_for_line(&line, now);
                }
                if self.config.discreet_mode {
                    self.agent.receipt = Some(format!("Stream mode: {}", mode.label()).into());
                } else {
                    self.notify(
                        format!("Auto reading mode: {}", mode.label()),
                        NotificationKind::Info,
                    );
                }
                vec![ReaderEffect::SaveConfig]
            }
            ReaderAction::CalibrateReadingSpeed => {
                let now = Instant::now();
                let chars = self.current_page_effective_chars();
                let is_chapter = self.is_current_page_chapter_start();
                match self
                    .auto_reading
                    .apply_calibrated_cpm(chars, is_chapter, now)
                {
                    Some(calibrated_cpm) => {
                        self.config.set_auto_reading_cpm(calibrated_cpm);
                        let count = self.auto_reading.calibration.sample_count();
                        if self.config.discreet_mode {
                            self.agent.receipt = Some(
                                format!("Stream cadence calibrated: {calibrated_cpm} CPM").into(),
                            );
                        } else {
                            self.notify(
                                format!("Calibrated reading speed: {calibrated_cpm} CPM (from {count} pages)"),
                                NotificationKind::Success,
                            );
                        }
                        vec![ReaderEffect::SaveConfig]
                    }
                    None => {
                        let count = self.auto_reading.calibration.sample_count();
                        if self.config.discreet_mode {
                            self.agent.receipt =
                                Some("Calibration requires manual reading samples".into());
                        } else {
                            self.notify(
                                format!(
                                    "Need 2-3 manually read pages to calibrate (currently {count})"
                                ),
                                NotificationKind::Info,
                            );
                        }
                        Vec::new()
                    }
                }
            }
            ReaderAction::AutoTick => self.handle_auto_tick(),
            ReaderAction::JumpStart => match &mut self.overlay {
                Overlay::Bookmarks(navigator) => {
                    navigator.selection = 0;
                    Vec::new()
                }
                _ => {
                    let effects = self.jump_boundary(false);
                    if self.auto_reading.active {
                        self.reset_auto_reading_for_current_page(Instant::now());
                    }
                    effects
                }
            },
            ReaderAction::JumpEnd => match &mut self.overlay {
                Overlay::Bookmarks(navigator) => {
                    if !self.bookmarks.is_empty() {
                        navigator.selection = self.bookmarks.len() - 1;
                    }
                    Vec::new()
                }
                _ => {
                    let effects = self.jump_boundary(true);
                    if self.auto_reading.active {
                        self.reset_auto_reading_for_current_page(Instant::now());
                    }
                    effects
                }
            },
            ReaderAction::OpenPercent => {
                self.overlay = Overlay::Percent(PercentNavigator::default());
                Vec::new()
            }
            ReaderAction::OpenContents => {
                self.overlay = Overlay::Contents(ContentsNavigator {
                    selection: contents_selection_for_position(
                        &self.document,
                        &self.metrics,
                        self.position,
                    ),
                    ..ContentsNavigator::default()
                });
                Vec::new()
            }
            ReaderAction::OpenSearch => {
                self.overlay = Overlay::Search(SearchInput::default());
                Vec::new()
            }
            ReaderAction::OpenSearchResults => {
                if self.search_results.is_empty() {
                    self.notify("No active search results", NotificationKind::Error);
                } else {
                    self.overlay = Overlay::SearchResults(SearchResultsNavigator {
                        selection: self.search_index,
                        error: None,
                    });
                }
                Vec::new()
            }
            ReaderAction::OpenBookmarks => {
                let selection = if self.bookmarks.is_empty() {
                    0
                } else {
                    self.bookmarks
                        .iter()
                        .position(|b| b.position >= self.position)
                        .unwrap_or(self.bookmarks.len().saturating_sub(1))
                };
                self.overlay = Overlay::Bookmarks(BookmarksNavigator {
                    selection,
                    error: None,
                });
                Vec::new()
            }
            ReaderAction::OpenMetadata => {
                self.overlay = Overlay::Metadata;
                Vec::new()
            }
            ReaderAction::OpenHelp => {
                if self.config.discreet_mode {
                    self.agent.help = true;
                    self.agent.help_offset = 0;
                    return Vec::new();
                }
                self.overlay = Overlay::Help;
                Vec::new()
            }
            ReaderAction::ToggleProgress => {
                self.config.toggle_progress();
                self.notify(
                    if self.config.show_progress {
                        "Progress shown"
                    } else {
                        "Progress hidden"
                    },
                    NotificationKind::Info,
                );
                vec![ReaderEffect::SaveConfig]
            }
            ReaderAction::ToggleDiscreet => {
                self.config.toggle_discreet_mode();
                self.overlay = Overlay::None;
                self.agent.command = None;
                self.agent.help = false;
                self.notification = None;
                self.repaginate();
                if self.config.discreet_mode {
                    self.agent.resumed = false;
                    self.agent.record_read(ReadDirection::Current);
                }
                self.notify(
                    if self.config.discreet_mode {
                        "Discreet mode on"
                    } else {
                        "Discreet mode off"
                    },
                    NotificationKind::Info,
                );
                vec![ReaderEffect::SaveConfig]
            }
            ReaderAction::ToggleHidden => {
                self.hidden = !self.hidden;
                self.overlay = Overlay::None;
                self.agent.command = None;
                self.agent.help = false;
                self.notification = None;
                let now = Instant::now();
                if self.hidden {
                    self.agent.receipt = None;
                    self.auto_reading.pause(now);
                } else {
                    self.auto_reading.resume(now);
                }
                if !self.hidden {
                    self.repaginate();
                }
                Vec::new()
            }
            ReaderAction::ToggleImmersive => {
                self.config.toggle_immersive_mode();
                self.repaginate();
                self.notify(
                    if self.config.immersive_mode {
                        "Immersive mode on"
                    } else {
                        "Immersive mode off"
                    },
                    NotificationKind::Info,
                );
                vec![ReaderEffect::SaveConfig]
            }
            ReaderAction::TogglePageOverlap => {
                self.config.toggle_page_overlap();
                self.repaginate();
                self.notify(
                    if self.config.page_overlap {
                        "Page overlap on"
                    } else {
                        "Page overlap off"
                    },
                    NotificationKind::Info,
                );
                vec![ReaderEffect::SaveConfig]
            }
            ReaderAction::AdjustTextWidth(delta) => {
                let default_width = if self.config.discreet_mode {
                    72
                } else {
                    self.viewport.width
                };
                let current = self.config.text_width.unwrap_or(default_width);
                let max_width = self.viewport.width.max(20);
                let new_width = if delta < 0 {
                    current.saturating_sub(delta.unsigned_abs()).max(20)
                } else {
                    current.saturating_add(delta.unsigned_abs()).min(max_width)
                };
                self.config.text_width = Some(new_width);
                self.repaginate();
                self.notify(format!("Text width: {new_width}"), NotificationKind::Info);
                vec![ReaderEffect::SaveConfig]
            }
            ReaderAction::ResetTextWidth => {
                self.config.text_width = None;
                self.repaginate();
                self.notify("Text width: auto", NotificationKind::Info);
                vec![ReaderEffect::SaveConfig]
            }
            ReaderAction::CycleTheme => {
                let theme = self.config.cycle_reading_theme();
                self.notify(format!("Theme: {}", theme.label()), NotificationKind::Info);
                vec![ReaderEffect::SaveConfig]
            }
            ReaderAction::SetReadingTheme(theme) => {
                self.config.set_reading_theme(theme);
                self.notify(format!("Theme: {}", theme.label()), NotificationKind::Info);
                vec![ReaderEffect::SaveConfig]
            }
            ReaderAction::AddBookmark => vec![ReaderEffect::AddBookmark {
                label: format!(
                    "{} {} · {}",
                    self.document.format.section_label(),
                    self.current_section() + 1,
                    format_tenths(self.progress_tenths())
                ),
                progress: self.current_progress(),
            }],
            ReaderAction::DeleteBookmark => {
                if let Overlay::Bookmarks(navigator) = &mut self.overlay {
                    if !self.bookmarks.is_empty() && navigator.selection < self.bookmarks.len() {
                        let index = navigator.selection;
                        let removed = self.bookmarks.remove(index);
                        if navigator.selection >= self.bookmarks.len() && !self.bookmarks.is_empty()
                        {
                            navigator.selection = self.bookmarks.len() - 1;
                        }
                        self.notify(
                            format!("Deleted bookmark: {}", removed.label),
                            NotificationKind::Info,
                        );
                        return vec![ReaderEffect::DeleteBookmark {
                            id: removed.id.clone(),
                            removed,
                            index,
                        }];
                    }
                }
                Vec::new()
            }
            ReaderAction::NextSearch => self.jump_search(1),
            ReaderAction::PreviousSearch => self.jump_search(-1),
            ReaderAction::Move(delta) => {
                self.move_overlay_selection(delta);
                Vec::new()
            }
            ReaderAction::Confirm => self.confirm_overlay(),
            ReaderAction::Cancel => {
                self.cancel_overlay();
                Vec::new()
            }
            ReaderAction::Backspace => {
                self.backspace_overlay();
                Vec::new()
            }
            ReaderAction::StartFilter => {
                if let Overlay::Contents(navigator) = &mut self.overlay {
                    navigator.filter_active = true;
                    navigator.filter_query.clear();
                    navigator.number_input.clear();
                    navigator.error = None;
                }
                Vec::new()
            }
            ReaderAction::Input(ch) => {
                self.input_overlay(ch);
                Vec::new()
            }
            ReaderAction::DiscreetUnavailable => {
                self.notify("Unavailable in discreet mode", NotificationKind::Info);
                Vec::new()
            }
        }
    }

    pub fn visible_contents(&self, navigator: &ContentsNavigator) -> Vec<usize> {
        visible_contents(&self.document, navigator)
    }

    fn move_screen(&mut self, delta: isize) -> Vec<ReaderEffect> {
        if self.config.discreet_mode {
            let height = self.agent_body_height();
            let overlap = usize::from(self.config.page_overlap && height >= 4);
            return self.scroll_agent(
                delta * height.saturating_sub(overlap).max(1) as isize,
                delta > 0 && overlap > 0,
            );
        }
        if !matches!(self.overlay, Overlay::None) {
            return Vec::new();
        }
        let old = self.current_page;
        let last = self.pages.len().saturating_sub(1) as isize;
        let next = (old as isize + delta).clamp(0, last) as usize;
        if next == old {
            return Vec::new();
        }
        self.set_page(next);
        vec![ReaderEffect::SaveProgress(self.current_progress())]
    }

    fn jump_boundary(&mut self, end: bool) -> Vec<ReaderEffect> {
        match &mut self.overlay {
            Overlay::None => {
                let before = self.position;
                let tenths = if end { 1000 } else { 0 };
                self.agent.view.context_rows = 0;
                self.jump_to_position(self.metrics.position_at_tenths(tenths));
                if self.position == before {
                    return Vec::new();
                }
                if self.config.discreet_mode {
                    self.agent.record_read(if end {
                        ReadDirection::Forward
                    } else {
                        ReadDirection::Backward
                    });
                }
                vec![ReaderEffect::SaveProgress(self.current_progress())]
            }
            Overlay::Contents(navigator) => {
                let visible = visible_contents(&self.document, navigator);
                if let Some(&selection) = if end { visible.last() } else { visible.first() } {
                    navigator.selection = selection;
                    navigator.number_input.clear();
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn set_page(&mut self, page_index: usize) {
        self.current_page = page_index.min(self.pages.len().saturating_sub(1));
        if let Some(page) = self.pages.get(self.current_page) {
            self.position = page.source_start;
        }
        self.line_scroll_offset = 0;
        self.reflow_agent();
    }

    fn jump_to_position(&mut self, position: TextPosition) {
        self.position = self.metrics.clamp(position);
        self.current_page = page_for_position(&self.pages, self.position);
        self.line_scroll_offset = 0;
        self.reflow_agent();
    }

    fn repaginate(&mut self) {
        self.pages = paginate_for_config(&self.document, self.viewport, &self.config);
        self.current_page = page_for_position(&self.pages, self.position);
        self.line_scroll_offset = 0;
        self.reflow_agent();
        let now = Instant::now();
        if self.auto_reading.active {
            self.reset_auto_reading_for_current_page(now);
        }
    }

    pub fn current_page_effective_chars(&self) -> usize {
        if self.config.discreet_mode {
            let height = u16::try_from(self.agent_body_height()).unwrap_or(u16::MAX);
            let visible = self.agent.view.visible(height);
            let lines: Vec<String> = visible.iter().map(|vl| vl.line.text.clone()).collect();
            crate::auto_reading::count_effective_reading_chars(&lines, self.agent.view.context_rows)
        } else {
            self.pages
                .get(self.current_page)
                .map(|p| {
                    crate::auto_reading::count_effective_reading_chars(&p.lines, p.overlap_lines)
                })
                .unwrap_or(0)
        }
    }

    pub fn is_current_page_chapter_start(&self) -> bool {
        if self.config.discreet_mode {
            self.agent.view.top == 0 || self.position.char_offset == 0
        } else {
            self.current_page == 0
                || self
                    .pages
                    .get(self.current_page)
                    .map(|p| p.source_start.char_offset == 0)
                    .unwrap_or(false)
        }
    }

    pub fn current_line_text(&self) -> Option<String> {
        if self.config.discreet_mode {
            self.agent
                .view
                .lines
                .get(self.agent.view.top)
                .map(|vl| vl.line.text.clone())
        } else {
            self.pages
                .get(self.current_page)
                .and_then(|p| p.lines.get(self.line_scroll_offset))
                .cloned()
        }
    }

    pub fn record_manual_page_turn(&mut self, now: Instant) {
        let chars = self.current_page_effective_chars();
        self.auto_reading.calibration.on_manual_page_turn(now);
        self.auto_reading.calibration.on_page_entered(chars, now);
    }

    pub fn reset_auto_reading_for_current_page(&mut self, now: Instant) {
        let chars = self.current_page_effective_chars();
        let is_chapter = self.is_current_page_chapter_start();
        self.auto_reading.reset_for_page(chars, is_chapter, now);
        if let Some(line) = self.current_line_text() {
            self.auto_reading.reset_for_line(&line, now);
        }
    }

    pub fn auto_reading_tick(&mut self, now: Instant) -> Option<ReaderAction> {
        if self.hidden
            || !matches!(self.overlay, Overlay::None)
            || self.agent.command.is_some()
            || self.agent.help
            || !self.auto_reading.active
            || self.auto_reading.paused
        {
            return None;
        }

        let is_due = match self.auto_reading.mode {
            crate::auto_reading::AutoReadingMode::TimedPage => self.auto_reading.is_page_due(now),
            crate::auto_reading::AutoReadingMode::LineScroll => self.auto_reading.is_line_due(now),
        };
        is_due.then_some(ReaderAction::AutoTick)
    }

    fn stop_auto_reading_at_end(&mut self) -> Vec<ReaderEffect> {
        self.auto_reading.stop();
        if self.config.discreet_mode {
            self.agent.receipt = Some("Stream completed".into());
        } else {
            self.notify("Reached end of document", NotificationKind::Info);
        }
        Vec::new()
    }

    fn scroll_lines_standard(&mut self, delta: isize) -> Vec<ReaderEffect> {
        if self.pages.is_empty() {
            return Vec::new();
        }
        let current_page_lines = self
            .pages
            .get(self.current_page)
            .map(|p| p.lines.len())
            .unwrap_or(0);
        let new_offset = self.line_scroll_offset as isize + delta;
        if new_offset < 0 {
            if self.current_page > 0 {
                self.current_page -= 1;
                let prev_page_lines = self
                    .pages
                    .get(self.current_page)
                    .map(|p| p.lines.len())
                    .unwrap_or(1);
                self.line_scroll_offset = (prev_page_lines as isize + new_offset).max(0) as usize;
            } else {
                self.line_scroll_offset = 0;
            }
        } else if new_offset >= current_page_lines as isize {
            if self.current_page + 1 < self.pages.len() {
                self.current_page += 1;
                self.line_scroll_offset =
                    (new_offset - current_page_lines as isize).max(0) as usize;
            } else {
                self.line_scroll_offset = current_page_lines.saturating_sub(1);
            }
        } else {
            self.line_scroll_offset = new_offset as usize;
        }

        if let Some(page) = self.pages.get(self.current_page) {
            if let Some(anchored) = page.anchored_lines.get(self.line_scroll_offset) {
                self.position = TextPosition::new(page.section_index, anchored.start_char);
            } else {
                self.position = page.source_start;
            }
        }
        vec![ReaderEffect::SaveProgress(self.current_progress())]
    }

    fn handle_auto_tick(&mut self) -> Vec<ReaderEffect> {
        let now = Instant::now();
        match self.auto_reading.mode {
            crate::auto_reading::AutoReadingMode::TimedPage => {
                let at_last = if self.config.discreet_mode {
                    let height = self.agent_body_height();
                    let view = &self.agent.view;
                    view.top.saturating_add(height) >= view.lines.len()
                } else {
                    self.current_page + 1 >= self.pages.len()
                };
                if at_last {
                    return self.stop_auto_reading_at_end();
                }
                let effects = self.move_screen(1);
                let chars = self.current_page_effective_chars();
                let is_chapter = self.is_current_page_chapter_start();
                self.auto_reading.reset_for_page(chars, is_chapter, now);
                effects
            }
            crate::auto_reading::AutoReadingMode::LineScroll => {
                if self.config.discreet_mode {
                    let view = &self.agent.view;
                    if view.top + 1 >= view.lines.len() {
                        return self.stop_auto_reading_at_end();
                    }
                    let effects = self.scroll_agent(1, false);
                    if let Some(line) = self.current_line_text() {
                        self.auto_reading.reset_for_line(&line, now);
                    }
                    effects
                } else {
                    let page_line_count = self
                        .pages
                        .get(self.current_page)
                        .map(|p| p.lines.len())
                        .unwrap_or(0);
                    if self.line_scroll_offset + 1 < page_line_count {
                        self.line_scroll_offset += 1;
                        if let Some(line) = self.current_line_text() {
                            self.auto_reading.reset_for_line(&line, now);
                        }
                        if let Some(page) = self.pages.get(self.current_page) {
                            if let Some(anchored) = page.anchored_lines.get(self.line_scroll_offset)
                            {
                                self.position =
                                    TextPosition::new(page.section_index, anchored.start_char);
                            }
                        }
                        vec![ReaderEffect::SaveProgress(self.current_progress())]
                    } else {
                        if self.current_page + 1 >= self.pages.len() {
                            return self.stop_auto_reading_at_end();
                        }
                        self.set_page(self.current_page + 1);
                        if let Some(line) = self.current_line_text() {
                            self.auto_reading.reset_for_line(&line, now);
                        }
                        vec![ReaderEffect::SaveProgress(self.current_progress())]
                    }
                }
            }
        }
    }

    fn reflow_agent(&mut self) {
        if self.config.discreet_mode {
            let layout = AgentLayout::with_options(
                self.viewport,
                self.config.text_width,
                self.config.agent_lines,
            );
            if layout.body.width == 0 || layout.body.height == 0 {
                return;
            }
            if layout.body.height < 4 {
                self.agent.view.context_rows = 0;
            }
            self.agent.view.reflow(
                &self.document.sections,
                layout.net_text_width(self.agent.density),
                self.position,
            );
        }
    }

    fn agent_body_height(&self) -> usize {
        usize::from(
            AgentLayout::with_options(
                self.viewport,
                self.config.text_width,
                self.config.agent_lines,
            )
            .net_text_height(),
        )
    }

    fn scroll_agent(&mut self, delta: isize, overlap: bool) -> Vec<ReaderEffect> {
        let height = self.agent_body_height();
        if self.agent.help {
            let count = crate::reader_input::DISCREET_BINDINGS.len()
                + crate::reader_input::LOCAL_COMMANDS.len()
                + 2;
            self.agent.help_offset = self
                .agent
                .help_offset
                .saturating_add_signed(delta)
                .min(count.saturating_sub(height));
            return Vec::new();
        }
        if !self.config.discreet_mode
            || height == 0
            || self.agent.help
            || self.agent.command.is_some()
        {
            return Vec::new();
        }
        let view = &mut self.agent.view;
        if view.lines.is_empty() {
            return Vec::new();
        }
        let old = view.top;
        // At a short final window, forward navigation has no new content to reveal.
        if delta > 0 && old.saturating_add(height) >= view.lines.len() {
            return Vec::new();
        }
        let mut top = old.saturating_add_signed(delta).min(view.lines.len() - 1);
        let mut context = usize::from(overlap && top + 1 < view.lines.len());
        if context > 0 && view.lines[top].section != view.lines[top + 1].section {
            top += 1;
            context = 0;
        }
        if top == old {
            return Vec::new();
        }
        view.top = top;
        view.context_rows = context;
        self.position = view.lines[top + context].position();
        self.agent.record_read(if delta > 0 {
            ReadDirection::Forward
        } else {
            ReadDirection::Backward
        });
        vec![ReaderEffect::SaveProgress(self.current_progress())]
    }

    fn update_command(&mut self, action: ReaderAction) -> Vec<ReaderEffect> {
        let Some(input) = self.agent.command.as_mut() else {
            return Vec::new();
        };
        match action {
            ReaderAction::Input(ch) if !ch.is_control() && input.chars().count() < 64 => {
                input.push(ch)
            }
            ReaderAction::Backspace => {
                input.pop();
            }
            ReaderAction::Cancel => {
                self.agent.command = None;
            }
            ReaderAction::CompleteCommand => {
                let matches: Vec<_> = crate::reader_input::LOCAL_COMMANDS
                    .iter()
                    .filter(|(name, _)| name.starts_with(input.as_str()))
                    .collect();
                if matches.len() == 1 {
                    *input = matches[0].0.to_string();
                }
            }
            ReaderAction::Confirm => {
                let text = self.agent.command.take().unwrap_or_default();
                let trimmed = text.trim();
                if let Some(arg) = trimmed
                    .strip_prefix("lines")
                    .or_else(|| trimmed.strip_prefix("agent-lines"))
                {
                    let arg = arg.trim();
                    if arg.is_empty() || arg == "auto" || arg == "0" {
                        return self.update(ReaderAction::SetAgentLines(None));
                    } else if let Ok(n) = arg.parse::<u16>() {
                        return self.update(ReaderAction::SetAgentLines(Some(n)));
                    } else {
                        self.agent.receipt = Some("Usage: :lines <N> or :lines auto".into());
                        return Vec::new();
                    }
                }
                if let Some(arg) = trimmed.strip_prefix("auto") {
                    let arg = arg.trim();
                    if arg.is_empty() {
                        return self.update(ReaderAction::ToggleAutoReading);
                    } else if arg == "off" || arg == "stop" {
                        return self.update(ReaderAction::StopAutoReading);
                    } else if arg == "info" || arg == "status" {
                        let cpm = self.auto_reading.cpm;
                        let mode_str = self.auto_reading.mode.label();
                        let state_str = if !self.auto_reading.active {
                            "off"
                        } else if self.auto_reading.paused {
                            "paused"
                        } else {
                            "active"
                        };
                        if self.config.discreet_mode {
                            self.agent.receipt = Some(
                                format!("Stream pace: {cpm} CPM ({mode_str} · {state_str})").into(),
                            );
                        } else {
                            self.notify(
                                format!("Auto reading: {cpm} CPM ({mode_str} · {state_str})"),
                                NotificationKind::Info,
                            );
                        }
                        return Vec::new();
                    } else if let Ok(cpm) = arg.parse::<u32>() {
                        self.update(ReaderAction::SetAutoReadingSpeed(cpm));
                        if !self.auto_reading.active {
                            return self.update(ReaderAction::ToggleAutoReading);
                        }
                        return Vec::new();
                    } else {
                        self.agent.receipt = Some("Usage: :auto [CPM|off|status]".into());
                        return Vec::new();
                    }
                }
                if let Some(arg) = trimmed
                    .strip_prefix("cpm")
                    .or_else(|| trimmed.strip_prefix("speed"))
                {
                    let arg = arg.trim();
                    if arg.is_empty() {
                        let cpm = self.auto_reading.cpm;
                        let mode_str = self.auto_reading.mode.label();
                        if self.config.discreet_mode {
                            self.agent.receipt =
                                Some(format!("Current stream pace: {cpm} CPM ({mode_str})").into());
                        } else {
                            self.notify(
                                format!("Current reading speed: {cpm} CPM ({mode_str})"),
                                NotificationKind::Info,
                            );
                        }
                        return Vec::new();
                    } else if let Ok(cpm) = arg.parse::<u32>() {
                        return self.update(ReaderAction::SetAutoReadingSpeed(cpm));
                    } else {
                        self.agent.receipt = Some("Usage: :cpm [number]".into());
                        return Vec::new();
                    }
                }
                if let Some(arg) = trimmed.strip_prefix("mode") {
                    let arg = arg.trim();
                    if arg.is_empty() {
                        let mode_str = self.auto_reading.mode.label();
                        if self.config.discreet_mode {
                            self.agent.receipt =
                                Some(format!("Current stream mode: {mode_str}").into());
                        } else {
                            self.notify(
                                format!("Current auto reading mode: {mode_str}"),
                                NotificationKind::Info,
                            );
                        }
                        return Vec::new();
                    } else if arg == "page" {
                        return self.update(ReaderAction::SetAutoReadingMode(
                            crate::auto_reading::AutoReadingMode::TimedPage,
                        ));
                    } else if arg == "scroll" || arg == "line" {
                        return self.update(ReaderAction::SetAutoReadingMode(
                            crate::auto_reading::AutoReadingMode::LineScroll,
                        ));
                    } else {
                        self.agent.receipt = Some("Usage: :mode <page|scroll>".into());
                        return Vec::new();
                    }
                }
                let action = crate::reader_input::LOCAL_COMMANDS
                    .iter()
                    .find(|(name, _)| *name == trimmed)
                    .map(|(_, action)| *action);
                if let Some(action) = action {
                    return self.update(action);
                }
                self.agent.receipt = Some("Unknown local command · :help".into());
            }
            _ => {}
        }
        Vec::new()
    }

    fn move_overlay_selection(&mut self, delta: isize) {
        match &mut self.overlay {
            Overlay::Contents(navigator) => {
                let visible = visible_contents(&self.document, navigator);
                if visible.is_empty() {
                    return;
                }
                let current = visible
                    .iter()
                    .position(|index| *index == navigator.selection)
                    .unwrap_or(0);
                let next = (current as isize + delta).clamp(0, visible.len() as isize - 1) as usize;
                navigator.selection = visible[next];
                navigator.number_input.clear();
                navigator.error = None;
            }
            Overlay::SearchResults(navigator) => {
                if self.search_results.is_empty() {
                    return;
                }
                let count = self.search_results.len() as isize;
                let next = (navigator.selection as isize + delta).clamp(0, count - 1) as usize;
                navigator.selection = next;
                navigator.error = None;
            }
            Overlay::Bookmarks(navigator) => {
                if self.bookmarks.is_empty() {
                    return;
                }
                let count = self.bookmarks.len() as isize;
                let next = (navigator.selection as isize + delta).clamp(0, count - 1) as usize;
                navigator.selection = next;
                navigator.error = None;
            }
            _ => {}
        }
    }

    fn confirm_overlay(&mut self) -> Vec<ReaderEffect> {
        match &self.overlay {
            Overlay::Percent(navigator) => match parse_percent_tenths(&navigator.input) {
                Ok(tenths) => {
                    self.jump_to_position(self.metrics.position_at_tenths(tenths));
                    self.overlay = Overlay::None;
                    self.notify(
                        format!("Jumped to {}", format_tenths(tenths)),
                        NotificationKind::Success,
                    );
                    vec![ReaderEffect::SaveProgress(self.current_progress())]
                }
                Err(error) => {
                    if let Overlay::Percent(navigator) = &mut self.overlay {
                        navigator.error = Some(error);
                    }
                    Vec::new()
                }
            },
            Overlay::Contents(navigator) => {
                if !navigator.filter_active && !navigator.number_input.is_empty() {
                    let valid_number = navigator
                        .number_input
                        .parse::<usize>()
                        .is_ok_and(|number| (1..=self.document.contents.len()).contains(&number));
                    if !valid_number {
                        if let Overlay::Contents(navigator) = &mut self.overlay {
                            navigator.error = Some(format!(
                                "Entry must be between 1 and {}",
                                self.document.contents.len()
                            ));
                        }
                        return Vec::new();
                    }
                }
                let visible = visible_contents(&self.document, navigator);
                if visible.contains(&navigator.selection) {
                    let selection = navigator.selection;
                    let entry = self.document.contents[selection].clone();
                    self.jump_to_position(entry.target);
                    self.overlay = Overlay::None;
                    self.notify(
                        format!("Jumped to contents {}: {}", selection + 1, entry.title),
                        NotificationKind::Success,
                    );
                    return vec![ReaderEffect::SaveProgress(self.current_progress())];
                }
                if let Overlay::Contents(navigator) = &mut self.overlay {
                    navigator.error = Some("No matching contents entry".to_string());
                }
                Vec::new()
            }
            Overlay::SearchResults(navigator) => {
                if !self.search_results.is_empty()
                    && navigator.selection < self.search_results.len()
                {
                    self.search_index = navigator.selection;
                    self.jump_to_position(self.search_results[self.search_index].position);
                    self.overlay = Overlay::None;
                    let query_label = self.search_query.as_deref().unwrap_or("");
                    self.notify(
                        format!(
                            "Match {}/{} for {:?}",
                            self.search_index + 1,
                            self.search_results.len(),
                            query_label
                        ),
                        NotificationKind::Info,
                    );
                    return vec![ReaderEffect::SaveProgress(self.current_progress())];
                }
                self.overlay = Overlay::None;
                Vec::new()
            }
            Overlay::Bookmarks(navigator) => {
                if !self.bookmarks.is_empty() && navigator.selection < self.bookmarks.len() {
                    let bookmark = self.bookmarks[navigator.selection].clone();
                    self.jump_to_position(bookmark.position);
                    self.overlay = Overlay::None;
                    self.notify(
                        format!("Jumped to bookmark: {}", bookmark.label),
                        NotificationKind::Success,
                    );
                    return vec![ReaderEffect::SaveProgress(self.current_progress())];
                }
                self.overlay = Overlay::None;
                Vec::new()
            }
            Overlay::Search(input) => {
                let query = input.query.trim().to_string();
                if query.is_empty() {
                    if let Overlay::Search(input) = &mut self.overlay {
                        input.error = Some("Enter a search term".to_string());
                    }
                    return Vec::new();
                }
                let results = self.document.search(&query);
                if results.is_empty() {
                    if let Overlay::Search(input) = &mut self.overlay {
                        input.error = Some(format!("No matches for {query:?}"));
                    }
                    return Vec::new();
                }
                self.search_results = results;
                self.search_query = Some(query.clone());
                self.search_index = 0;
                self.jump_to_position(self.search_results[0].position);
                self.overlay = Overlay::SearchResults(SearchResultsNavigator {
                    selection: 0,
                    error: None,
                });
                self.notify(
                    format!("Match 1/{} for {query:?}", self.search_results.len()),
                    NotificationKind::Success,
                );
                vec![ReaderEffect::SaveProgress(self.current_progress())]
            }
            Overlay::Metadata | Overlay::Help => {
                self.overlay = Overlay::None;
                Vec::new()
            }
            Overlay::None => Vec::new(),
        }
    }

    fn cancel_overlay(&mut self) {
        if let Overlay::Contents(navigator) = &mut self.overlay {
            if navigator.filter_active {
                navigator.filter_active = false;
                navigator.filter_query.clear();
                navigator.error = None;
                return;
            }
        }
        self.overlay = Overlay::None;
    }

    fn backspace_overlay(&mut self) {
        match &mut self.overlay {
            Overlay::Percent(navigator) => {
                navigator.input.pop();
                navigator.error = None;
            }
            Overlay::Contents(navigator) if navigator.filter_active => {
                navigator.filter_query.pop();
                navigator.error = None;
                sync_contents_selection(&self.document, navigator);
            }
            Overlay::Contents(navigator) => {
                navigator.number_input.pop();
                navigator.error = None;
                sync_number_selection(self.document.contents.len(), navigator);
            }
            Overlay::Search(input) => {
                input.query.pop();
                input.error = None;
            }
            _ => {}
        }
    }

    fn input_overlay(&mut self, ch: char) {
        match &mut self.overlay {
            Overlay::Percent(navigator) => {
                if ch.is_ascii_digit() || ch == '.' {
                    navigator.input.push(ch);
                    navigator.error = None;
                } else {
                    navigator.error = Some("Use a number from 0 to 100".to_string());
                }
            }
            Overlay::Contents(navigator) if navigator.filter_active && !ch.is_control() => {
                navigator.filter_query.push(ch);
                navigator.error = None;
                sync_contents_selection(&self.document, navigator);
            }
            Overlay::Contents(navigator) => {
                if ch.is_ascii_digit() {
                    navigator.number_input.push(ch);
                    navigator.error = None;
                    sync_number_selection(self.document.contents.len(), navigator);
                } else {
                    navigator.error =
                        Some("Type a contents entry number or press / to filter".to_string());
                }
            }
            Overlay::Search(input) if !ch.is_control() => {
                input.query.push(ch);
                input.error = None;
            }
            _ => {}
        }
    }

    fn jump_search(&mut self, delta: isize) -> Vec<ReaderEffect> {
        if self.search_results.is_empty() {
            self.notify("No active search results", NotificationKind::Error);
            return Vec::new();
        }
        let length = self.search_results.len() as isize;
        self.search_index = (self.search_index as isize + delta).rem_euclid(length) as usize;
        self.jump_to_position(self.search_results[self.search_index].position);
        if let Overlay::SearchResults(navigator) = &mut self.overlay {
            navigator.selection = self.search_index;
        }
        let query_label = self.search_query.as_deref().unwrap_or("");
        self.notify(
            format!(
                "Match {}/{} for {:?}",
                self.search_index + 1,
                self.search_results.len(),
                query_label
            ),
            NotificationKind::Info,
        );
        vec![ReaderEffect::SaveProgress(self.current_progress())]
    }
}

pub(crate) fn format_tenths(tenths: u16) -> String {
    format!("{}.{:01}%", tenths / 10, tenths % 10)
}

pub(crate) fn parse_percent_tenths(input: &str) -> Result<u16, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("Enter a percentage from 0 to 100".to_string());
    }
    let mut parts = input.split('.');
    let whole = parts
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| "Enter a percentage from 0 to 100".to_string())?;
    let fraction = match parts.next() {
        None => 0,
        Some("") => 0,
        Some(value) if value.len() == 1 => value
            .parse::<u16>()
            .map_err(|_| "Use at most one decimal place".to_string())?,
        Some(_) => return Err("Use at most one decimal place".to_string()),
    };
    if parts.next().is_some() || whole > 100 || (whole == 100 && fraction > 0) {
        return Err("Percentage must be between 0 and 100".to_string());
    }
    Ok(whole * 10 + fraction)
}

fn paginate_for_config(
    document: &BookDocument,
    viewport: Rect,
    config: &ReaderConfig,
) -> Vec<Page> {
    if config.discreet_mode {
        return Vec::new();
    }
    let content_height = if config.immersive_mode {
        viewport.height.max(1)
    } else {
        viewport.height.saturating_sub(2).max(1)
    };
    paginate_with_options(
        &document.sections,
        viewport.width,
        content_height,
        config.text_width,
        config.natural_breaks,
        config.page_overlap,
    )
}

fn action_is_available_in_discreet(action: ReaderAction) -> bool {
    matches!(
        action,
        ReaderAction::None
            | ReaderAction::Exit
            | ReaderAction::NextScreen
            | ReaderAction::PreviousScreen
            | ReaderAction::JumpStart
            | ReaderAction::JumpEnd
            | ReaderAction::ToggleDiscreet
            | ReaderAction::ToggleHidden
            | ReaderAction::AddBookmark
            | ReaderAction::DiscreetUnavailable
            | ReaderAction::ScrollLines(_)
            | ReaderAction::OpenCommand
            | ReaderAction::CompleteCommand
            | ReaderAction::OpenHelp
            | ReaderAction::AdjustTextWidth(_)
            | ReaderAction::ResetTextWidth
            | ReaderAction::TogglePageOverlap
            | ReaderAction::ToggleDensity
            | ReaderAction::SetAgentLines(_)
            | ReaderAction::CycleTheme
            | ReaderAction::SetReadingTheme(_)
            | ReaderAction::ToggleAutoReading
            | ReaderAction::ToggleAutoReadingPause
            | ReaderAction::StopAutoReading
            | ReaderAction::AdjustAutoReadingSpeed(_)
            | ReaderAction::SetAutoReadingSpeed(_)
            | ReaderAction::SetAutoReadingMode(_)
            | ReaderAction::CalibrateReadingSpeed
            | ReaderAction::AutoTick
    )
}

fn visible_contents(document: &BookDocument, navigator: &ContentsNavigator) -> Vec<usize> {
    if !navigator.filter_active || navigator.filter_query.is_empty() {
        return (0..document.contents.len()).collect();
    }
    let needle = navigator.filter_query.to_lowercase();
    document
        .contents
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            entry
                .title
                .to_lowercase()
                .contains(&needle)
                .then_some(index)
        })
        .collect()
}

fn sync_contents_selection(document: &BookDocument, navigator: &mut ContentsNavigator) {
    let visible = visible_contents(document, navigator);
    if !visible.contains(&navigator.selection) {
        navigator.selection = visible.first().copied().unwrap_or(0);
    }
}

fn sync_number_selection(entry_count: usize, navigator: &mut ContentsNavigator) {
    if navigator.number_input.is_empty() {
        return;
    }
    match navigator.number_input.parse::<usize>() {
        Ok(number) if (1..=entry_count).contains(&number) => {
            navigator.selection = number - 1;
        }
        _ => navigator.error = Some(format!("Entry must be between 1 and {entry_count}")),
    }
}

fn contents_selection_for_position(
    document: &BookDocument,
    metrics: &ContentMetrics,
    position: TextPosition,
) -> usize {
    let current_offset = metrics.global_char_offset(position);
    document
        .contents
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let target_offset = metrics.global_char_offset(entry.target);
            (target_offset <= current_offset).then_some((target_offset, index))
        })
        .max()
        .map(|(_, index)| index)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::book::{BookFormat, BookMetadata, ContentsEntry, ContentsSource, Section};

    fn state() -> ReaderState {
        let sections = vec![
            Section {
                title: "Opening".into(),
                text: "alpha beta gamma delta epsilon".into(),
            },
            Section {
                title: "Second Rust Chapter".into(),
                text: "one two three four five six seven eight".into(),
            },
        ];
        let document = BookDocument {
            id: "book".into(),
            title: "Book".into(),
            path: "book.txt".into(),
            format: BookFormat::Txt,
            metadata: BookMetadata {
                title: "Book".into(),
                author: None,
                language: None,
                publisher: None,
                format: BookFormat::Txt,
                path: "book.txt".into(),
                section_count: sections.len(),
            },
            sections,
            contents: vec![
                ContentsEntry {
                    title: "Opening".into(),
                    depth: 0,
                    target: TextPosition::new(0, 0),
                },
                ContentsEntry {
                    title: "Second Rust Chapter".into(),
                    depth: 1,
                    target: TextPosition::new(1, 4),
                },
            ],
            contents_source: ContentsSource::Document,
        };
        ReaderState::new(
            document,
            "book".into(),
            Rect::new(0, 0, 24, 8),
            ReaderConfig::default(),
            Progress::default(),
            Vec::new(),
        )
    }

    #[test]
    fn parses_percentage_boundaries_and_one_decimal() {
        assert_eq!(parse_percent_tenths("0"), Ok(0));
        assert_eq!(parse_percent_tenths("37.4"), Ok(374));
        assert_eq!(parse_percent_tenths("100"), Ok(1000));
        assert!(parse_percent_tenths("100.1").is_err());
        assert!(parse_percent_tenths("1.25").is_err());
    }

    #[test]
    fn resize_preserves_stable_position() {
        let mut state = state();
        state.jump_to_position(TextPosition::new(1, 12));
        let before = state.position;
        state.resize(80, 24);
        assert_eq!(state.position, before);
    }

    #[test]
    fn percent_confirm_jumps_and_closes_overlay() {
        let mut state = state();
        state.update(ReaderAction::OpenPercent);
        state.update(ReaderAction::Input('5'));
        state.update(ReaderAction::Input('0'));
        let effects = state.update(ReaderAction::Confirm);
        assert!(matches!(state.overlay, Overlay::None));
        assert!(matches!(
            effects.as_slice(),
            [ReaderEffect::SaveProgress(_)]
        ));
        assert!((490..=510).contains(&state.progress_tenths()));
    }

    #[test]
    fn contents_number_and_filter_select_exact_targets() {
        let mut state = state();
        state.update(ReaderAction::OpenContents);
        state.update(ReaderAction::Input('2'));
        state.update(ReaderAction::Confirm);
        assert_eq!(state.position, TextPosition::new(1, 4));

        state.update(ReaderAction::OpenContents);
        state.update(ReaderAction::StartFilter);
        for ch in "rust".chars() {
            state.update(ReaderAction::Input(ch));
        }
        let Overlay::Contents(navigator) = &state.overlay else {
            panic!("contents overlay expected");
        };
        assert_eq!(state.visible_contents(navigator), vec![1]);
    }

    #[test]
    fn contents_open_selects_the_nearest_preceding_target() {
        let mut state = state();
        state.jump_to_position(TextPosition::new(1, 12));

        state.update(ReaderAction::OpenContents);

        let Overlay::Contents(navigator) = &state.overlay else {
            panic!("contents overlay expected");
        };
        assert_eq!(navigator.selection, 1);
    }

    #[test]
    fn first_escape_clears_filter_and_second_closes_navigator() {
        let mut state = state();
        state.update(ReaderAction::OpenContents);
        state.update(ReaderAction::StartFilter);
        state.update(ReaderAction::Input('x'));
        state.update(ReaderAction::Cancel);
        assert!(matches!(
            state.overlay,
            Overlay::Contents(ContentsNavigator {
                filter_active: false,
                ..
            })
        ));
        state.update(ReaderAction::Cancel);
        assert!(matches!(state.overlay, Overlay::None));
    }

    #[test]
    fn invalid_contents_number_keeps_navigator_open() {
        let mut state = state();
        state.update(ReaderAction::OpenContents);
        for ch in "99".chars() {
            state.update(ReaderAction::Input(ch));
        }

        assert!(state.update(ReaderAction::Confirm).is_empty());
        assert!(matches!(
            state.overlay,
            Overlay::Contents(ContentsNavigator { error: Some(_), .. })
        ));
        assert_eq!(state.current_section(), 0);
    }

    #[test]
    fn pdf_navigation_and_bookmarks_use_page_labels() {
        let mut state = state();
        state.document.format = BookFormat::Pdf;
        state.document.metadata.format = BookFormat::Pdf;

        let effects = state.update(ReaderAction::AddBookmark);
        assert!(matches!(
            effects.as_slice(),
            [ReaderEffect::AddBookmark { label, .. }] if label.starts_with("Page 1")
        ));

        state.update(ReaderAction::OpenContents);
        state.update(ReaderAction::Input('9'));
        state.update(ReaderAction::Confirm);
        let Overlay::Contents(navigator) = &state.overlay else {
            panic!("contents overlay expected");
        };
        assert_eq!(
            navigator.error.as_deref(),
            Some("Entry must be between 1 and 2")
        );
    }

    #[test]
    fn non_navigation_toggles_and_bookmarks_preserve_position() {
        let mut state = state();
        state.jump_to_position(TextPosition::new(1, 12));
        let before = state.position;

        assert!(matches!(
            state.update(ReaderAction::AddBookmark).as_slice(),
            [ReaderEffect::AddBookmark { .. }]
        ));
        state.update(ReaderAction::ToggleDiscreet);
        state.update(ReaderAction::ToggleDiscreet);
        state.update(ReaderAction::OpenSearch);
        state.update(ReaderAction::Cancel);

        assert_eq!(state.position, before);
    }

    #[test]
    fn hidden_mode_ignores_mutating_actions_and_restores_exact_state() {
        let mut state = state();
        state.update(ReaderAction::OpenSearch);
        state.update(ReaderAction::Input('x'));
        let position = state.position;

        assert!(state.update(ReaderAction::ToggleHidden).is_empty());
        assert!(state.hidden);
        assert!(state.update(ReaderAction::NextScreen).is_empty());
        assert!(state.update(ReaderAction::ToggleDiscreet).is_empty());
        assert!(state.update(ReaderAction::AddBookmark).is_empty());
        assert_eq!(state.position, position);
        assert!(!state.config.discreet_mode);
        assert!(matches!(state.overlay, Overlay::None));

        state.update(ReaderAction::ToggleHidden);
        assert!(!state.hidden);
        assert!(matches!(state.overlay, Overlay::None));
    }

    #[test]
    fn discreet_mode_rejects_overlay_and_progress_actions() {
        let mut state = state();
        state.update(ReaderAction::ToggleDiscreet);
        let progress_setting = state.config.show_progress;

        assert!(state.update(ReaderAction::OpenSearch).is_empty());
        assert!(state.update(ReaderAction::ToggleProgress).is_empty());

        assert!(matches!(state.overlay, Overlay::None));
        assert_eq!(state.config.show_progress, progress_setting);
        assert_eq!(
            state
                .notification
                .as_ref()
                .map(|notice| notice.text.as_str()),
            Some("Unavailable in discreet mode")
        );
    }

    fn agent_state(width: u16, height: u16) -> ReaderState {
        let mut s = state();
        s.document.sections = vec![Section {
            title: "private-section".into(),
            text: (0..100)
                .map(|n| format!("　　第{n}行中文正文，保留字符锚点。\n"))
                .collect(),
        }];
        s.metrics = s.document.content_metrics();
        s.resize(width, height);
        s.update(ReaderAction::ToggleDiscreet);
        s
    }

    #[test]
    fn agent_scroll_and_screen_overlap_use_source_rows() {
        let mut s = agent_state(80, 24);
        assert_eq!(s.physical_row_range(), (1, 7, 101));
        s.update(ReaderAction::ScrollLines(2));
        assert_eq!(s.agent.view.top, 2);
        assert_eq!(s.position, s.agent.view.lines[2].position());
        s.update(ReaderAction::NextScreen);
        // top 2 + (7 - 1) = 8
        assert_eq!(s.agent.view.top, 8);
        assert_eq!(s.agent.view.context_rows, 1);
        assert_eq!(s.position, s.agent.view.lines[9].position());
        s.update(ReaderAction::PreviousScreen);
        assert_eq!(s.agent.view.top, 2);
        assert_eq!(s.agent.view.context_rows, 0);
        s.update(ReaderAction::ScrollLines(-2));
        assert_eq!(s.agent.view.top, 0);
        let reads = s.agent.reads;
        assert!(s.update(ReaderAction::ScrollLines(-2)).is_empty());
        assert_eq!(s.agent.reads, reads);
    }

    #[test]
    fn agent_reflow_modes_and_folding_keep_exact_character_not_new_line_start() {
        let mut s = agent_state(80, 24);
        let anchor = TextPosition::new(0, 147);
        s.jump_to_position(anchor);
        let reads = s.agent.reads;
        for (w, h) in [(60, 18), (120, 30), (80, 24)] {
            s.resize(w, h);
            assert_eq!(s.position, anchor);
        }
        assert_eq!(s.agent.reads, reads);
        s.update(ReaderAction::ToggleHidden);
        s.resize(60, 18);
        s.update(ReaderAction::ToggleHidden);
        assert_eq!(s.position, anchor);
        assert_eq!(s.agent.reads, reads);
        s.update(ReaderAction::ToggleDiscreet);
        s.update(ReaderAction::ToggleDiscreet);
        assert_eq!(s.position, anchor);
        let top = &s.agent.view.lines[s.agent.view.top];
        assert!(
            top.line.start_char <= anchor.char_offset && top.line.end_char >= anchor.char_offset
        );
    }

    #[test]
    fn agent_checkpoint_waits_for_io_and_preserves_window() {
        let mut s = agent_state(60, 18);
        s.update(ReaderAction::ScrollLines(2));
        let top = s.agent.view.top;
        let reads = s.agent.reads;
        let effects = s.update(ReaderAction::AddBookmark);
        assert!(
            matches!(&effects[0],ReaderEffect::AddBookmark { progress,.. } if progress.position == s.position)
        );
        assert_eq!(s.agent.checkpoints, 0);
        assert_eq!(s.agent.receipt, None);
        assert_eq!((s.agent.view.top, s.agent.reads), (top, reads));
    }

    #[test]
    fn agent_commands_execute_only_registry_entries_and_f12_discards_drafts() {
        let mut s = agent_state(80, 24);
        s.update(ReaderAction::OpenCommand);
        for ch in "down".chars() {
            s.update(ReaderAction::Input(ch));
        }
        s.update(ReaderAction::Confirm);
        assert_eq!(s.agent.view.top, 2);
        s.update(ReaderAction::OpenCommand);
        for ch in "private arbitrary input".chars() {
            s.update(ReaderAction::Input(ch));
        }
        s.update(ReaderAction::Confirm);
        assert_eq!(
            s.agent.receipt.as_deref(),
            Some("Unknown local command · :help")
        );
        s.update(ReaderAction::OpenCommand);
        for _ in 0..100 {
            s.update(ReaderAction::Input('x'));
        }
        assert_eq!(s.agent.command.as_ref().unwrap().len(), 64);
        s.update(ReaderAction::ToggleHidden);
        assert!(s.agent.command.is_none());
        s.update(ReaderAction::ToggleHidden);
        assert!(s.agent.command.is_none());
        s.update(ReaderAction::OpenCommand);
        for ch in "check".chars() {
            s.update(ReaderAction::Input(ch));
        }
        s.update(ReaderAction::CompleteCommand);
        assert_eq!(s.agent.command.as_deref(), Some("checkpoint"));
    }

    #[test]
    fn agent_overlap_does_not_cross_section_boundary_or_repeat_at_end() {
        let mut s = state();
        s.document.sections = vec![
            Section {
                title: "one".into(),
                text: (0..12)
                    .map(|n| format!("{n}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            },
            Section {
                title: "two".into(),
                text: "next\nlast".into(),
            },
        ];
        s.metrics = s.document.content_metrics();
        s.resize(80, 24);
        s.update(ReaderAction::ToggleDiscreet);
        s.update(ReaderAction::NextScreen);
        // In 80x24 net text height is 7. Section 0 has 12 lines.
        // First NextScreen moves by 6 lines (to line 6 of section 0).
        assert_eq!(s.agent.view.top, 6);
        assert_eq!(s.position, TextPosition::new(0, 14));
        assert_eq!(s.agent.view.context_rows, 1);
        // Second NextScreen: moves from 6 by 6 -> top is 12 (which is section 1 line 0: "next")
        s.update(ReaderAction::NextScreen);
        assert_eq!(s.agent.view.top, 12);
        assert_eq!(s.position, TextPosition::new(1, 5));
        assert_eq!(s.agent.view.context_rows, 1);
        // Third NextScreen: top 12 + height 7 = 19 >= view.lines.len() (14). At end, does not repeat.
        let reads = s.agent.reads;
        s.update(ReaderAction::NextScreen);
        assert_eq!(s.agent.reads, reads);
    }

    #[test]
    fn physical_row_range_is_one_based_continuous_and_counts_blank_rows() {
        let mut state = state();
        state.pages = vec![
            Page {
                index: 0,
                section_index: 0,
                source_start: TextPosition::new(0, 0),
                source_end: TextPosition::new(0, 2),
                lines: vec!["first".into(), String::new(), "third".into()],
                anchored_lines: Vec::new(),
                overlap_lines: 0,
            },
            Page {
                index: 1,
                section_index: 0,
                source_start: TextPosition::new(0, 2),
                source_end: TextPosition::new(0, 4),
                lines: vec!["fourth".into(), "fifth".into()],
                anchored_lines: Vec::new(),
                overlap_lines: 0,
            },
        ];

        state.current_page = 0;
        assert_eq!(state.physical_row_range(), (1, 3, 5));
        state.current_page = 1;
        assert_eq!(state.physical_row_range(), (4, 5, 5));
    }

    #[test]
    fn entering_last_screen_does_not_mark_book_complete() {
        let mut state = state();
        while state.current_page + 1 < state.pages.len() {
            state.update(ReaderAction::NextScreen);
        }

        assert_eq!(state.position, state.pages.last().unwrap().source_start);
        assert!(state.progress_tenths() < 1000);

        state.update(ReaderAction::JumpEnd);
        assert_eq!(state.progress_tenths(), 1000);
    }

    #[test]
    fn search_workflow_navigates_and_wraps_results() {
        let mut state = state();
        state.update(ReaderAction::OpenSearch);
        for ch in "ta".chars() {
            state.update(ReaderAction::Input(ch));
        }
        state.update(ReaderAction::Confirm);

        assert!(matches!(
            state.overlay,
            Overlay::SearchResults(SearchResultsNavigator { selection: 0, .. })
        ));
        assert_eq!(state.search_results.len(), 2);
        assert_eq!(state.search_index, 0);

        state.update(ReaderAction::Cancel);
        assert!(matches!(state.overlay, Overlay::None));

        state.update(ReaderAction::OpenSearchResults);
        assert!(matches!(
            state.overlay,
            Overlay::SearchResults(SearchResultsNavigator { selection: 0, .. })
        ));
        state.update(ReaderAction::Move(1));
        assert!(matches!(
            state.overlay,
            Overlay::SearchResults(SearchResultsNavigator { selection: 1, .. })
        ));
        state.update(ReaderAction::Confirm);
        assert!(matches!(state.overlay, Overlay::None));
        assert_eq!(state.search_index, 1);

        state.update(ReaderAction::NextSearch);
        assert_eq!(state.search_index, 0);
        state.update(ReaderAction::PreviousSearch);
        assert_eq!(state.search_index, 1);
    }

    #[test]
    fn text_width_adjustment_and_auto_reset() {
        let mut state = state();
        assert_eq!(state.config.text_width, None);

        let initial_pos = state.position;
        let effects = state.update(ReaderAction::AdjustTextWidth(-2));
        assert_eq!(effects, vec![ReaderEffect::SaveConfig]);
        assert_eq!(state.config.text_width, Some(22)); // 24 - 2 = 22
        assert_eq!(state.position, initial_pos);

        // Adjust down to min clamp (20)
        state.update(ReaderAction::AdjustTextWidth(-4));
        assert_eq!(state.config.text_width, Some(20));

        // Adjust up
        state.update(ReaderAction::AdjustTextWidth(2));
        assert_eq!(state.config.text_width, Some(22));

        // Reset to auto
        let effects_reset = state.update(ReaderAction::ResetTextWidth);
        assert_eq!(effects_reset, vec![ReaderEffect::SaveConfig]);
        assert_eq!(state.config.text_width, None);
        assert_eq!(state.position, initial_pos);
    }

    #[test]
    fn immersive_mode_toggle_allocates_full_height_and_preserves_position() {
        let mut state = state();
        assert!(!state.config.immersive_mode);
        // Default height is 8, minus 2 for header/footer = 6 rows in normal mode
        let normal_page_lines = state.pages[0].lines.len();

        let initial_pos = state.position;
        let effects = state.update(ReaderAction::ToggleImmersive);
        assert_eq!(effects, vec![ReaderEffect::SaveConfig]);
        assert!(state.config.immersive_mode);
        assert_eq!(state.position, initial_pos);

        // In immersive mode, height 8 is fully available for content
        // Given enough lines, immersive mode fits more lines per page
        // Let's verify that repaginate was executed with immersive height
        let effects_toggle_back = state.update(ReaderAction::ToggleImmersive);
        assert_eq!(effects_toggle_back, vec![ReaderEffect::SaveConfig]);
        assert!(!state.config.immersive_mode);
        assert_eq!(state.pages[0].lines.len(), normal_page_lines);
        assert_eq!(state.position, initial_pos);
    }

    #[test]
    fn bookmarks_workflow_navigates_jumps_and_deletes() {
        let mut state = state();
        state.bookmarks = vec![
            Bookmark {
                id: "bm1".into(),
                label: "Bookmark 1".into(),
                position: TextPosition::new(0, 5),
            },
            Bookmark {
                id: "bm2".into(),
                label: "Bookmark 2".into(),
                position: TextPosition::new(1, 10),
            },
        ];

        // Open bookmarks overlay
        state.update(ReaderAction::OpenBookmarks);
        assert!(matches!(
            state.overlay,
            Overlay::Bookmarks(BookmarksNavigator { selection: 0, .. })
        ));

        // Navigate down to item 1
        state.update(ReaderAction::Move(1));
        assert!(matches!(
            state.overlay,
            Overlay::Bookmarks(BookmarksNavigator { selection: 1, .. })
        ));

        // Confirm jump to Bookmark 2
        let effects = state.update(ReaderAction::Confirm);
        assert!(matches!(state.overlay, Overlay::None));
        assert_eq!(state.position, TextPosition::new(1, 10));
        assert!(matches!(
            effects.as_slice(),
            [ReaderEffect::SaveProgress(_)]
        ));

        // Re-open and delete Bookmark 2
        state.update(ReaderAction::OpenBookmarks);
        assert!(matches!(
            state.overlay,
            Overlay::Bookmarks(BookmarksNavigator { selection: 1, .. })
        ));
        let delete_effects = state.update(ReaderAction::DeleteBookmark);
        assert_eq!(
            delete_effects,
            vec![ReaderEffect::DeleteBookmark {
                id: "bm2".into(),
                removed: Bookmark {
                    id: "bm2".into(),
                    label: "Bookmark 2".into(),
                    position: TextPosition::new(1, 10),
                },
                index: 1,
            }]
        );
        assert_eq!(state.bookmarks.len(), 1);
        assert!(matches!(
            state.overlay,
            Overlay::Bookmarks(BookmarksNavigator { selection: 0, .. })
        ));
    }

    #[test]
    fn page_overlap_toggle_repaginates_and_preserves_position() {
        let mut state = state();
        assert!(state.config.page_overlap);
        let pos = state.position;

        let effects = state.update(ReaderAction::TogglePageOverlap);
        assert_eq!(effects, vec![ReaderEffect::SaveConfig]);
        assert!(!state.config.page_overlap);
        assert_eq!(state.position, pos);

        let effects_back = state.update(ReaderAction::TogglePageOverlap);
        assert_eq!(effects_back, vec![ReaderEffect::SaveConfig]);
        assert!(state.config.page_overlap);
        assert_eq!(state.position, pos);
    }

    #[test]
    fn reading_theme_cycle_and_set_updates_config_and_notifies() {
        let mut state = state();
        assert_eq!(
            state.config.reading_theme,
            crate::ui::ReadingTheme::Terminal
        );

        let effects = state.update(ReaderAction::CycleTheme);
        assert_eq!(effects, vec![ReaderEffect::SaveConfig]);
        assert_eq!(state.config.reading_theme, crate::ui::ReadingTheme::Sepia);
        assert!(state
            .notification
            .as_ref()
            .unwrap()
            .text
            .contains("Warm Sepia"));

        let effects_set =
            state.update(ReaderAction::SetReadingTheme(crate::ui::ReadingTheme::Nord));
        assert_eq!(effects_set, vec![ReaderEffect::SaveConfig]);
        assert_eq!(state.config.reading_theme, crate::ui::ReadingTheme::Nord);
        assert!(state
            .notification
            .as_ref()
            .unwrap()
            .text
            .contains("Nord Dark"));
    }

    #[test]
    fn auto_reading_page_mode_workflow_and_tick() {
        let mut state = state();
        assert!(!state.auto_reading.active);

        // Turn on auto reading
        state.update(ReaderAction::ToggleAutoReading);
        assert!(state.auto_reading.active);
        assert!(!state.auto_reading.paused);
        assert_eq!(state.auto_reading.cpm, 300);
        assert_eq!(state.current_page, 0);

        // Advance tick: should move to page 1
        let effects = state.update(ReaderAction::AutoTick);
        assert_eq!(state.current_page, 1);
        assert!(effects
            .iter()
            .any(|e| matches!(e, ReaderEffect::SaveProgress(_))));

        // When reaching the end, it gracefully deactivates
        while state.current_page + 1 < state.pages.len() {
            state.update(ReaderAction::AutoTick);
        }
        assert_eq!(state.current_page, state.pages.len() - 1);
        // Next tick at the end stops auto reading
        state.update(ReaderAction::AutoTick);
        assert!(!state.auto_reading.active);
    }

    #[test]
    fn auto_reading_pause_resume_and_speed_adjust() {
        let mut state = state();
        state.update(ReaderAction::ToggleAutoReading);
        assert!(state.auto_reading.active && !state.auto_reading.paused);

        // 'a' (ToggleAutoReading) pauses when active
        state.update(ReaderAction::ToggleAutoReading);
        assert!(state.auto_reading.paused);

        // 'a' resumes when paused
        state.update(ReaderAction::ToggleAutoReading);
        assert!(!state.auto_reading.paused);

        // Adjust speed
        let effects = state.update(ReaderAction::AdjustAutoReadingSpeed(50));
        assert_eq!(effects, vec![ReaderEffect::SaveConfig]);
        assert_eq!(state.auto_reading.cpm, 350);
        assert_eq!(state.config.auto_reading_cpm(), 350);

        // Set speed directly
        state.update(ReaderAction::SetAutoReadingSpeed(400));
        assert_eq!(state.auto_reading.cpm, 400);
        assert_eq!(state.config.auto_reading_cpm(), 400);

        // Stop auto reading via StopAutoReading
        state.update(ReaderAction::StopAutoReading);
        assert!(!state.auto_reading.active);
    }

    #[test]
    fn auto_reading_line_scroll_mode() {
        let mut state = state();
        state.update(ReaderAction::SetAutoReadingMode(
            crate::auto_reading::AutoReadingMode::LineScroll,
        ));
        assert_eq!(
            state.auto_reading.mode,
            crate::auto_reading::AutoReadingMode::LineScroll
        );
        state.update(ReaderAction::ToggleAutoReading);
        assert!(state.auto_reading.active);

        // Initial line offset is 0
        assert_eq!(state.line_scroll_offset, 0);

        // Auto tick advances line offset
        state.update(ReaderAction::AutoTick);
        assert_eq!(state.line_scroll_offset, 1);
    }

    #[test]
    fn auto_reading_calibration_flow() {
        let mut state = state();
        // Calibrate with no samples: informs user
        state.update(ReaderAction::CalibrateReadingSpeed);
        assert!(state
            .notification
            .as_ref()
            .unwrap()
            .text
            .contains("Need 2-3 manually read pages"));

        // Simulate 3 manual page reads
        let now = Instant::now();
        state.auto_reading.calibration.on_page_entered(300, now);
        state
            .auto_reading
            .calibration
            .on_manual_page_turn(now + std::time::Duration::from_secs(60)); // 300 CPM

        let now2 = now + std::time::Duration::from_secs(60);
        state.auto_reading.calibration.on_page_entered(400, now2);
        state
            .auto_reading
            .calibration
            .on_manual_page_turn(now2 + std::time::Duration::from_secs(60)); // 400 CPM

        let now3 = now2 + std::time::Duration::from_secs(60);
        state.auto_reading.calibration.on_page_entered(350, now3);
        state
            .auto_reading
            .calibration
            .on_manual_page_turn(now3 + std::time::Duration::from_secs(60)); // 350 CPM

        // Now calibrate: median is 350, 90% is 315 CPM!
        let effects = state.update(ReaderAction::CalibrateReadingSpeed);
        assert_eq!(effects, vec![ReaderEffect::SaveConfig]);
        assert_eq!(state.auto_reading.cpm, 315);
        assert_eq!(state.config.auto_reading_cpm(), 315);
        assert!(state
            .notification
            .as_ref()
            .unwrap()
            .text
            .contains("315 CPM"));
    }

    #[test]
    fn discreet_auto_reading_commands() {
        let mut state = state();
        state.update(ReaderAction::ToggleDiscreet);
        assert!(state.config.discreet_mode);

        // Command: :auto 320
        state.update(ReaderAction::OpenCommand);
        for ch in "auto 320".chars() {
            state.update(ReaderAction::Input(ch));
        }
        state.update(ReaderAction::Confirm);
        assert!(state.auto_reading.active);
        assert_eq!(state.auto_reading.cpm, 320);
        assert_eq!(
            state.agent.receipt.as_deref(),
            Some("Stream pace: 320 CPM (Page)")
        );

        // Command: :cpm (query)
        state.update(ReaderAction::OpenCommand);
        for ch in "cpm".chars() {
            state.update(ReaderAction::Input(ch));
        }
        state.update(ReaderAction::Confirm);
        assert_eq!(
            state.agent.receipt.as_deref(),
            Some("Current stream pace: 320 CPM (Page)")
        );

        // Command: :pause
        state.update(ReaderAction::OpenCommand);
        for ch in "pause".chars() {
            state.update(ReaderAction::Input(ch));
        }
        state.update(ReaderAction::Confirm);
        assert!(state.auto_reading.paused);
        assert_eq!(
            state.agent.receipt.as_deref(),
            Some("Stream pace: paused (320 CPM)")
        );

        // Command: :resume
        state.update(ReaderAction::OpenCommand);
        for ch in "resume".chars() {
            state.update(ReaderAction::Input(ch));
        }
        state.update(ReaderAction::Confirm);
        assert!(!state.auto_reading.paused);
        assert_eq!(state.agent.receipt.as_deref(), Some("Stream pace: 320 CPM"));

        // Command: :auto off
        state.update(ReaderAction::OpenCommand);
        for ch in "auto off".chars() {
            state.update(ReaderAction::Input(ch));
        }
        state.update(ReaderAction::Confirm);
        assert!(!state.auto_reading.active);
        assert_eq!(state.agent.receipt.as_deref(), Some("Stream pace: off"));
    }
}
