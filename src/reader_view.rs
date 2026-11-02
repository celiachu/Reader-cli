use std::time::Instant;

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::book::ContentsSource;
use crate::discreet::{AgentDensity, AgentLayout, AgentViewMode};
use crate::reader_input::{DISCREET_BINDINGS, LOCAL_COMMANDS, READING_BINDINGS};
use crate::reader_state::{
    format_tenths, ContentsNavigator, NotificationKind, Overlay, ReaderState,
};
use crate::ui::{centered_rect, fill_background, help_lines, hud_chunks, Palette};

pub(crate) fn draw(frame: &mut ratatui::Frame<'_>, state: &ReaderState) {
    let palette = if state.config.discreet_mode {
        Palette::for_reading_theme(state.config.reading_theme).discreet_mode()
    } else {
        Palette::for_reading_theme(state.config.reading_theme)
    };
    fill_background(frame, palette);
    if state.config.discreet_mode {
        draw_discreet(frame, state, palette);
        return;
    }

    let body_area = if state.config.immersive_mode {
        frame.area()
    } else {
        let [status_area, body_area, action_area] = hud_chunks(frame.area());
        if state.hidden {
            let neutral_status = if frame.area().width < 50 {
                Line::from(vec![
                    Span::styled("cargo test", palette.accent()),
                    Span::styled(" · ", palette.muted()),
                    Span::styled("Output folded", palette.muted()),
                ])
            } else {
                Line::from(vec![
                    Span::styled("terminal", palette.muted()),
                    Span::styled(" · ", palette.muted()),
                    Span::styled("cargo test --workspace --lib", palette.accent()),
                    Span::styled(" · ", palette.muted()),
                    Span::styled("Output folded", palette.muted()),
                ])
            };
            frame.render_widget(
                Paragraph::new(neutral_status)
                    .style(palette.base())
                    .alignment(Alignment::Center),
                status_area,
            );
            let neutral_action = if frame.area().width < 45 {
                Line::from(vec![
                    Span::styled("[ok] ", palette.accent()),
                    Span::styled("38 passed", palette.body()),
                    Span::styled(" · ", palette.muted()),
                    Span::styled("[F12 resume]", palette.muted()),
                ])
            } else {
                Line::from(vec![
                    Span::styled(" [ok] ", palette.accent()),
                    Span::styled("cargo-test · 38 passed · exit code 0", palette.body()),
                    Span::styled(" · ", palette.muted()),
                    Span::styled("[F12 resume]", palette.muted()),
                ])
            };
            frame.render_widget(
                Paragraph::new(neutral_action).style(palette.base()),
                action_area,
            );
        } else {
            frame.render_widget(
                Paragraph::new(status_line(state, palette))
                    .style(palette.base())
                    .alignment(Alignment::Center),
                status_area,
            );
            frame.render_widget(
                Paragraph::new(context_action_line(state, palette)).style(palette.base()),
                action_area,
            );
        }
        body_area
    };

    let text_area = reading_text_area(body_area, state.config.text_width);
    if state.hidden {
        frame.render_widget(
            Paragraph::new(simulated_tool_lines(usize::from(text_area.height), palette))
                .style(palette.body()),
            text_area,
        );
        return;
    }

    frame.render_widget(
        Paragraph::new(render_reading_lines(state, palette)).style(palette.body()),
        text_area,
    );

    if let Some(notification) = state.notification.as_ref() {
        draw_notification(
            frame,
            body_area,
            notification.text.as_str(),
            notification.kind,
            palette,
        );
    }

    match &state.overlay {
        Overlay::None => {}
        Overlay::Percent(navigator) => draw_percent(frame, navigator, palette),
        Overlay::Contents(navigator) => draw_contents(frame, state, navigator, palette),
        Overlay::Search(input) => draw_search(frame, input, palette),
        Overlay::SearchResults(navigator) => draw_search_results(frame, state, navigator, palette),
        Overlay::Bookmarks(navigator) => draw_bookmarks(frame, state, navigator, palette),
        Overlay::Metadata => draw_metadata(frame, state, palette),
        Overlay::Help => draw_help(frame, palette),
    }
}

fn status_line(state: &ReaderState, palette: Palette) -> Line<'static> {
    let section_index = state.current_section();
    let section_count = state.document.sections.len();
    let section_label = state.document.format.section_label();
    let section_title = state
        .document
        .sections
        .get(section_index)
        .map(|section| section.title.as_str())
        .unwrap_or("Untitled");

    let width = state.viewport.width;
    if width < 52 {
        return Line::from(vec![
            Span::styled(truncate_width(&state.document.title, 18), palette.accent()),
            Span::styled(
                format!("  {}/{}", section_index + 1, section_count.max(1)),
                palette.muted(),
            ),
        ]);
    }

    let spans = vec![
        Span::styled(
            format!("《{}》", truncate_width(&state.document.title, 26)),
            palette.accent(),
        ),
        Span::styled("   ·   ", palette.muted()),
        Span::styled(
            format!(
                "{section_label} {}/{}: {}",
                section_index + 1,
                section_count.max(1),
                truncate_width(section_title, 34)
            ),
            palette.body(),
        ),
    ];
    Line::from(spans)
}

fn reading_footer_line(state: &ReaderState, palette: Palette) -> Line<'static> {
    let width = usize::from(state.viewport.width);
    let tenths = state.progress_tenths();
    let percent_str = format_tenths(tenths);
    let section_idx = state.current_section();
    let section_count = state.document.sections.len();
    let section_info = format!("{}/{}", section_idx + 1, section_count.max(1));

    let auto_status = if state.auto_reading.active {
        let now = Instant::now();
        if state.auto_reading.paused {
            format!("[Auto: Paused · {} CPM] ", state.auto_reading.cpm)
        } else {
            let rem_secs = state.auto_reading.remaining_time(now).as_secs();
            match state.auto_reading.mode {
                crate::auto_reading::AutoReadingMode::TimedPage => {
                    format!("[Auto: {} CPM · {}s] ", state.auto_reading.cpm, rem_secs)
                }
                crate::auto_reading::AutoReadingMode::LineScroll => {
                    format!("[Auto: {} CPM · Scroll] ", state.auto_reading.cpm)
                }
            }
        }
    } else {
        String::new()
    };

    if width < 54 {
        let mut spans = vec![
            Span::styled(format!(" {percent_str} "), palette.accent()),
            Span::styled("· ", palette.muted()),
            Span::styled(format!("Sec {section_info}"), palette.body()),
        ];
        if state.auto_reading.active {
            let cpm = state.auto_reading.cpm;
            let flag = if state.auto_reading.paused { 'P' } else { 'A' };
            let tag = if width < 42 {
                format!(" [{flag}:{cpm}]")
            } else {
                format!(" [{flag}: {cpm} CPM]")
            };
            spans.push(Span::styled(tag, palette.accent()));
        } else {
            spans.push(Span::styled("  [?]", palette.muted()));
        }
        return Line::from(spans);
    }

    let left_spans = vec![Span::styled(format!(" {percent_str} "), palette.accent())];
    let mut right_spans = Vec::new();
    if !auto_status.is_empty() {
        right_spans.push(Span::styled(auto_status.clone(), palette.accent()));
    }
    right_spans.push(Span::styled(
        format!("Section {section_info}   "),
        palette.body(),
    ));
    right_spans.push(Span::styled("[? help] ", palette.muted()));

    let left_width = percent_str.width() + 2;
    let right_width = section_info.width() + 19 + auto_status.width();
    let available_bar = width.saturating_sub(left_width + right_width + 4);

    if available_bar >= 8 {
        let ratio = (tenths as f32 / 1000.0).clamp(0.0, 1.0);
        let filled_chars = (ratio * available_bar as f32).round() as usize;
        let filled_chars = filled_chars.min(available_bar);
        let empty_chars = available_bar.saturating_sub(filled_chars);

        let mut spans = left_spans;
        spans.push(Span::styled("  ", palette.muted()));
        if filled_chars > 0 {
            let solid = filled_chars.saturating_sub(1);
            if solid > 0 {
                spans.push(Span::styled("━".repeat(solid), palette.accent()));
            }
            spans.push(Span::styled("╸", palette.accent()));
        }
        if empty_chars > 0 {
            spans.push(Span::styled("━".repeat(empty_chars), palette.muted()));
        }
        spans.push(Span::styled("   ", palette.muted()));
        spans.extend(right_spans);
        Line::from(spans)
    } else {
        split_row_line(left_spans, right_spans, state.viewport.width)
    }
}

fn context_action_line(state: &ReaderState, palette: Palette) -> Line<'static> {
    match &state.overlay {
        Overlay::None => reading_footer_line(state, palette),
        Overlay::Percent(_) => prompt_actions(
            &[("Enter", "Jump"), ("Backspace", "Edit"), ("Esc", "Cancel")],
            palette,
        ),
        Overlay::Contents(navigator) if navigator.filter_active => prompt_actions(
            &[
                ("Type", "Filter"),
                ("Up/Down", "Select"),
                ("Enter", "Jump"),
                ("Esc", "Clear"),
            ],
            palette,
        ),
        Overlay::Contents(_) => prompt_actions(
            &[
                ("j/k", "Select"),
                ("0-9", "Number"),
                ("/", "Filter"),
                ("Enter", "Jump"),
                ("Esc", "Cancel"),
            ],
            palette,
        ),
        Overlay::Search(_) => prompt_actions(
            &[
                ("Enter", "Search"),
                ("Backspace", "Edit"),
                ("Esc", "Cancel"),
            ],
            palette,
        ),
        Overlay::SearchResults(_) => prompt_actions(
            &[
                ("j/k", "Select"),
                ("Enter", "Jump"),
                ("n/N", "Next/Prev"),
                ("Esc", "Close"),
            ],
            palette,
        ),
        Overlay::Bookmarks(_) => prompt_actions(
            &[
                ("j/k", "Select"),
                ("Enter", "Jump"),
                ("d", "Delete"),
                ("Esc", "Close"),
            ],
            palette,
        ),
        Overlay::Metadata | Overlay::Help => prompt_actions(&[("Esc/Enter", "Close")], palette),
    }
}

fn prompt_actions(actions: &[(&str, &str)], palette: Palette) -> Line<'static> {
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

fn reading_text_area(area: Rect, text_width: Option<u16>) -> Rect {
    let available = area.width.saturating_sub(2).max(1);
    let width = text_width
        .map(|width| width.max(20).min(available))
        .unwrap_or_else(|| available.min(crate::pagination::DEFAULT_COMFORT_WIDTH as u16));
    Rect::new(
        area.x + (area.width.saturating_sub(width)) / 2,
        area.y,
        width,
        area.height,
    )
}

fn draw_notification(
    frame: &mut ratatui::Frame<'_>,
    body_area: Rect,
    text: &str,
    kind: NotificationKind,
    palette: Palette,
) {
    if body_area.height == 0 {
        return;
    }
    let width = (text.width() as u16 + 4).min(body_area.width.max(1));
    let area = Rect::new(
        body_area.right().saturating_sub(width),
        body_area.bottom().saturating_sub(1),
        width,
        1,
    );
    frame.render_widget(Clear, area);
    let style = match kind {
        NotificationKind::Info => palette.accent(),
        NotificationKind::Success => palette.success(),
        NotificationKind::Error => palette.error(),
    };
    frame.render_widget(
        Paragraph::new(format!("  {text}  ")).style(style.bg(palette.background)),
        area,
    );
}

fn popup_block<'a>(title: &'a str, palette: Palette) -> Block<'a> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(palette.border())
        .style(palette.popup())
        .padding(Padding::horizontal(1))
}

fn draw_percent(
    frame: &mut ratatui::Frame<'_>,
    navigator: &crate::reader_state::PercentNavigator,
    palette: Palette,
) {
    let area = centered_rect(56, 34, frame.area());
    frame.render_widget(Clear, area);
    let mut lines = vec![
        Line::from(Span::styled(
            "Jump to a stable position in the book.",
            palette.muted(),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Percentage: ", palette.body()),
            Span::styled(format!("{}%", navigator.input), palette.accent()),
        ]),
    ];
    if let Some(error) = navigator.error.as_deref() {
        lines.push(Line::from(Span::styled(error.to_string(), palette.error())));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Enter confirms · Esc cancels · Range 0–100, one decimal",
        palette.muted(),
    )));
    frame.render_widget(
        Paragraph::new(lines)
            .block(popup_block(" Go to percentage ", palette))
            .style(palette.popup())
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_contents(
    frame: &mut ratatui::Frame<'_>,
    state: &ReaderState,
    navigator: &ContentsNavigator,
    palette: Palette,
) {
    let area = centered_rect(78, 78, frame.area());
    frame.render_widget(Clear, area);
    let title = if state.document.contents_source == ContentsSource::Generated {
        " Contents (generated) "
    } else {
        " Contents "
    };
    frame.render_widget(popup_block(title, palette), area);
    let inner = area.inner(ratatui::layout::Margin {
        horizontal: 2,
        vertical: 1,
    });
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);

    let prompt = if navigator.filter_active {
        Line::from(vec![
            Span::styled("Filter: /", palette.body()),
            Span::styled(navigator.filter_query.clone(), palette.accent()),
        ])
    } else {
        Line::from(vec![
            Span::styled("Entry number: ", palette.body()),
            Span::styled(navigator.number_input.clone(), palette.accent()),
        ])
    };
    let count = state.visible_contents(navigator).len();
    frame.render_widget(
        Paragraph::new(vec![
            prompt,
            Line::from(Span::styled(
                format!("{count} matching entries"),
                palette.muted(),
            )),
        ])
        .style(palette.popup()),
        chunks[0],
    );

    let visible = state.visible_contents(navigator);
    let max_items = chunks[1].height as usize;
    let selected_row = visible
        .iter()
        .position(|index| *index == navigator.selection)
        .unwrap_or(0);
    let start = selected_row.saturating_sub(max_items.saturating_sub(1) / 2);
    let lines = visible
        .iter()
        .skip(start)
        .take(max_items)
        .map(|&entry_index| {
            let entry = &state.document.contents[entry_index];
            let indentation = "  ".repeat(entry.depth.min(12));
            let text = format!(" {:>4}  {indentation}{} ", entry_index + 1, entry.title);
            if entry_index == navigator.selection {
                Line::from(Span::styled(text, palette.selected()))
            } else {
                Line::from(Span::styled(text, palette.body()))
            }
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines).style(palette.popup()), chunks[1]);

    let footer = navigator
        .error
        .as_deref()
        .map(|error| Line::from(Span::styled(error.to_string(), palette.error())))
        .unwrap_or_else(|| {
            Line::from(Span::styled(
                if navigator.filter_active {
                    "Esc clears filter · Enter jumps"
                } else {
                    "/ filters titles · Enter jumps · Esc closes"
                },
                palette.muted(),
            ))
        });
    frame.render_widget(Paragraph::new(footer).style(palette.popup()), chunks[2]);
}

fn draw_search(
    frame: &mut ratatui::Frame<'_>,
    input: &crate::reader_state::SearchInput,
    palette: Palette,
) {
    let area = centered_rect(62, 32, frame.area());
    frame.render_widget(Clear, area);
    let mut lines = vec![Line::from(vec![
        Span::styled("/", palette.accent()),
        Span::styled(input.query.clone(), palette.body()),
    ])];
    if let Some(error) = input.error.as_deref() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(error.to_string(), palette.error())));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Enter searches · Esc cancels · n/N moves through results",
        palette.muted(),
    )));
    frame.render_widget(
        Paragraph::new(lines)
            .block(popup_block(" Search ", palette))
            .style(palette.popup()),
        area,
    );
}

fn render_reading_lines(state: &ReaderState, palette: Palette) -> Vec<Line<'static>> {
    let page = &state.pages[state.current_page];
    let offset = state.line_scroll_offset.min(page.lines.len());
    let overlap_count = page.overlap_lines;
    let relevant_matches: Vec<(usize, usize, usize)> = if state.search_results.is_empty() {
        Vec::new()
    } else {
        state
            .search_results
            .iter()
            .enumerate()
            .filter(|(_, result)| result.position.section_index == page.section_index)
            .map(|(idx, result)| {
                (
                    idx,
                    result.position.char_offset,
                    result.end_position.char_offset,
                )
            })
            .collect()
    };

    if relevant_matches.is_empty() {
        let mut lines: Vec<Line<'static>> = page
            .lines
            .iter()
            .enumerate()
            .skip(offset)
            .map(|(idx, line)| {
                let style = if idx < overlap_count {
                    palette.muted()
                } else {
                    palette.body()
                };
                Line::from(Span::styled(line.clone(), style))
            })
            .collect();
        if offset > 0 && state.current_page + 1 < state.pages.len() {
            let next_page = &state.pages[state.current_page + 1];
            let remaining_slots = page.lines.len().saturating_sub(lines.len());
            for line in next_page.lines.iter().take(remaining_slots) {
                lines.push(Line::from(Span::styled(line.clone(), palette.body())));
            }
        }
        return lines;
    }

    let mut rendered_lines = Vec::with_capacity(page.anchored_lines.len());
    for (line_idx, line) in page.anchored_lines.iter().enumerate().skip(offset) {
        if line.text.is_empty() {
            rendered_lines.push(Line::from(""));
            continue;
        }

        let base_style = if line_idx < overlap_count {
            palette.muted()
        } else {
            palette.body()
        };

        let has_overlap = relevant_matches
            .iter()
            .any(|&(_, start, end)| start < line.end_char && end > line.start_char);

        if !has_overlap || line.char_offsets.is_empty() {
            rendered_lines.push(Line::from(Span::styled(line.text.clone(), base_style)));
            continue;
        }

        let chars: Vec<char> = line.text.chars().collect();
        let mut spans = Vec::new();
        let mut span_chars = String::new();
        let mut current_style = None;

        for (i, &ch) in chars.iter().enumerate() {
            let src_offset = line.char_offsets.get(i).copied().unwrap_or(line.start_char);
            let mut char_style = base_style;
            for &(match_idx, m_start, m_end) in &relevant_matches {
                if src_offset >= m_start && src_offset < m_end {
                    if match_idx == state.search_index {
                        char_style = palette.search_match_current();
                        break;
                    } else {
                        char_style = palette.search_match_secondary();
                    }
                }
            }

            match current_style {
                None => {
                    current_style = Some(char_style);
                    span_chars.push(ch);
                }
                Some(style) if style == char_style => {
                    span_chars.push(ch);
                }
                Some(style) => {
                    spans.push(Span::styled(std::mem::take(&mut span_chars), style));
                    current_style = Some(char_style);
                    span_chars.push(ch);
                }
            }
        }

        if !span_chars.is_empty() {
            spans.push(Span::styled(
                span_chars,
                current_style.unwrap_or(base_style),
            ));
        }

        rendered_lines.push(Line::from(spans));
    }

    rendered_lines
}

fn draw_search_results(
    frame: &mut ratatui::Frame<'_>,
    state: &ReaderState,
    navigator: &crate::reader_state::SearchResultsNavigator,
    palette: Palette,
) {
    let area = centered_rect(78, 68, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(popup_block(" Search Results ", palette), area);
    let inner = area.inner(ratatui::layout::Margin {
        horizontal: 2,
        vertical: 1,
    });
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);

    let query_label = state.search_query.as_deref().unwrap_or("");
    let count = state.search_results.len();
    let prompt = Line::from(vec![
        Span::styled("Query: /", palette.body()),
        Span::styled(query_label, palette.accent()),
    ]);
    frame.render_widget(
        Paragraph::new(vec![
            prompt,
            Line::from(Span::styled(
                format!(
                    "{count} matching results (Selected {}/{count})",
                    navigator.selection + 1
                ),
                palette.muted(),
            )),
        ])
        .style(palette.popup()),
        chunks[0],
    );

    let visible_items = chunks[1].height as usize;
    let selected_row = navigator.selection.min(count.saturating_sub(1));
    let start = selected_row.saturating_sub(visible_items.saturating_sub(1) / 2);
    let lines = state
        .search_results
        .iter()
        .enumerate()
        .skip(start)
        .take(visible_items)
        .map(|(idx, result)| {
            let section_idx = result.position.section_index;
            let section_title = state
                .document
                .sections
                .get(section_idx)
                .map(|s| s.title.as_str())
                .unwrap_or("Section");
            let text = format!(
                " {:>3}/{}  [{}]  {} ",
                idx + 1,
                count,
                section_title,
                result.preview
            );
            if idx == navigator.selection {
                Line::from(Span::styled(text, palette.selected()))
            } else {
                Line::from(Span::styled(text, palette.body()))
            }
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines).style(palette.popup()), chunks[1]);

    let footer = navigator
        .error
        .as_deref()
        .map(|err| Line::from(Span::styled(err.to_string(), palette.error())))
        .unwrap_or_else(|| {
            Line::from(Span::styled(
                "Enter jumps · j/k moves · Esc closes · n/N moves through results",
                palette.muted(),
            ))
        });
    frame.render_widget(Paragraph::new(footer).style(palette.popup()), chunks[2]);
}

fn bookmark_preview(
    document: &crate::book::BookDocument,
    position: crate::book::TextPosition,
) -> String {
    let Some(section) = document.sections.get(position.section_index) else {
        return String::new();
    };
    let chars: Vec<char> = section.text.chars().collect();
    let start = position.char_offset;
    let end = (start + 35).min(chars.len());
    if start >= chars.len() {
        return String::new();
    }
    let snippet: String = chars[start..end].iter().collect();
    snippet.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn draw_bookmarks(
    frame: &mut ratatui::Frame<'_>,
    state: &ReaderState,
    navigator: &crate::reader_state::BookmarksNavigator,
    palette: Palette,
) {
    let area = centered_rect(78, 68, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(popup_block(" Bookmarks ", palette), area);
    let inner = area.inner(ratatui::layout::Margin {
        horizontal: 2,
        vertical: 1,
    });
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);

    let count = state.bookmarks.len();
    let header = if count == 0 {
        Line::from(Span::styled("No saved bookmarks", palette.muted()))
    } else {
        Line::from(vec![Span::styled(
            format!(
                "{count} saved bookmark{} (Selected {}/{count})",
                if count == 1 { "" } else { "s" },
                navigator.selection + 1
            ),
            palette.muted(),
        )])
    };
    frame.render_widget(Paragraph::new(header).style(palette.popup()), chunks[0]);

    if count == 0 {
        let empty_msg = Paragraph::new(vec![
            Line::from(Span::styled("No bookmarks found.", palette.body())),
            Line::from(Span::styled(
                "Press 'b' during reading to bookmark the current position.",
                palette.muted(),
            )),
        ])
        .style(palette.popup());
        frame.render_widget(empty_msg, chunks[1]);
    } else {
        let visible_items = chunks[1].height as usize;
        let selected_row = navigator.selection.min(count.saturating_sub(1));
        let start = selected_row.saturating_sub(visible_items.saturating_sub(1) / 2);
        let lines = state
            .bookmarks
            .iter()
            .enumerate()
            .skip(start)
            .take(visible_items)
            .map(|(idx, bookmark)| {
                let section_idx = bookmark.position.section_index;
                let section_title = state
                    .document
                    .sections
                    .get(section_idx)
                    .map(|s| s.title.as_str())
                    .unwrap_or("Section");
                let pct = format_tenths(state.metrics.progress_tenths(bookmark.position));
                let preview = bookmark_preview(&state.document, bookmark.position);
                let preview_part = if preview.is_empty() {
                    String::new()
                } else {
                    format!(" · \"{preview}\"")
                };
                let text = format!(
                    " {:>2}. [{}] {} ({}){} ",
                    idx + 1,
                    section_title,
                    bookmark.label,
                    pct,
                    preview_part,
                );
                if idx == navigator.selection {
                    Line::from(Span::styled(text, palette.selected()))
                } else {
                    Line::from(Span::styled(text, palette.body()))
                }
            })
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(lines).style(palette.popup()), chunks[1]);
    }

    let footer = navigator
        .error
        .as_deref()
        .map(|err| Line::from(Span::styled(err.to_string(), palette.error())))
        .unwrap_or_else(|| {
            Line::from(Span::styled(
                "Enter jumps · j/k moves · d deletes · Esc closes",
                palette.muted(),
            ))
        });
    frame.render_widget(Paragraph::new(footer).style(palette.popup()), chunks[2]);
}

fn draw_metadata(frame: &mut ratatui::Frame<'_>, state: &ReaderState, palette: Palette) {
    let area = centered_rect(72, 62, frame.area());
    frame.render_widget(Clear, area);
    let metadata = &state.document.metadata;
    let mut lines = vec![
        metadata_line("Title", &metadata.title, palette),
        metadata_line("Format", metadata.format.label(), palette),
        metadata_line("Path", &metadata.path.display().to_string(), palette),
        metadata_line(
            metadata.format.section_label_plural(),
            &metadata.section_count.to_string(),
            palette,
        ),
    ];
    if let Some(author) = metadata.author.as_deref() {
        lines.push(metadata_line("Author", author, palette));
    }
    if let Some(language) = metadata.language.as_deref() {
        lines.push(metadata_line("Language", language, palette));
    }
    if let Some(publisher) = metadata.publisher.as_deref() {
        lines.push(metadata_line("Publisher", publisher, palette));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Esc or Enter closes",
        palette.muted(),
    )));
    frame.render_widget(
        Paragraph::new(lines)
            .block(popup_block(" Book information ", palette))
            .style(palette.popup())
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn metadata_line(label: &str, value: &str, palette: Palette) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label}: "), palette.muted()),
        Span::styled(value.to_string(), palette.body()),
    ])
}

fn draw_help(frame: &mut ratatui::Frame<'_>, palette: Palette) {
    let area = centered_rect(92, 90, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(popup_block(" Reader help ", palette), area);
    let inner = area.inner(ratatui::layout::Margin {
        horizontal: 2,
        vertical: 1,
    });
    let midpoint = READING_BINDINGS.len().div_ceil(2);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(inner);

    let mut left = help_lines("Reading", &READING_BINDINGS[..midpoint], palette);
    left.push(Line::from(""));
    left.push(Line::from(Span::styled(
        "Esc does nothing at the reading root.",
        palette.muted(),
    )));
    let mut right = help_lines(
        "More reading commands",
        &READING_BINDINGS[midpoint..],
        palette,
    );
    right.extend([
        Line::from(""),
        Line::from(Span::styled("Contents navigator", palette.accent())),
        Line::from(Span::styled(
            "  arrows/j/k browse · digits select · / filters",
            palette.body(),
        )),
        Line::from(Span::styled(
            "  first Esc clears a filter; second Esc closes",
            palette.body(),
        )),
        Line::from(""),
        Line::from(Span::styled("Esc or Enter closes help", palette.muted())),
    ]);
    frame.render_widget(Paragraph::new(left).style(palette.popup()), columns[0]);
    frame.render_widget(Paragraph::new(right).style(palette.popup()), columns[1]);
}

fn agent_symbol(symbol: &'static str, fallback: &'static str) -> &'static str {
    if std::env::var("TERM").is_ok_and(|term| term == "dumb") {
        fallback
    } else {
        symbol
    }
}

fn split_row_line(
    left_spans: Vec<Span<'static>>,
    right_spans: Vec<Span<'static>>,
    width: u16,
) -> Line<'static> {
    let left_len: usize = left_spans.iter().map(|s| s.content.width()).sum();
    let right_len: usize = right_spans.iter().map(|s| s.content.width()).sum();
    let pad = usize::from(width).saturating_sub(left_len + right_len);
    let mut spans = left_spans;
    if pad > 0 {
        spans.push(Span::raw(" ".repeat(pad)));
    }
    spans.extend(right_spans);
    Line::from(spans)
}

fn generate_agent_header_lines(
    target_height: usize,
    state: &ReaderState,
    palette: Palette,
    start: usize,
    end: usize,
) -> Vec<Line<'static>> {
    if target_height == 0 {
        return Vec::new();
    }
    let spinner_frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let now_millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let spin_idx = ((now_millis / 100) % (spinner_frames.len() as u128)) as usize;
    let spinner = agent_symbol(spinner_frames[spin_idx], "*");
    let section_count = state.document.sections.len();
    let current_section = state.current_section() + 1;
    let tip = match (now_millis / 4000) % 4 {
        0 => "_Tip: When fixing lifetime warnings, check reference scopes first.",
        1 => "_Tip: Pass `--fix` to automatically apply suggestions where supported.",
        2 => "_Tip: Use `cargo check --tests` to verify test harness coverage quickly.",
        _ => "_Tip: Keep data mutations localized to minimize lock contention.",
    };

    let inspect_line = Line::from(vec![
        Span::styled(format!("{} ", agent_symbol("❯", ">")), palette.accent()),
        Span::styled(
            "optimize stream ingestion and telemetry buffer pipeline",
            palette.body(),
        ),
    ]);
    let tool_call_line = Line::from(vec![
        Span::styled(format!("{} ", agent_symbol("●", "*")), palette.accent()),
        Span::styled(
            format!(r#"inspect_buffer(channel="stream_ingress", offset={start}, limit={end})"#),
            palette.body(),
        ),
    ]);
    let tool_res_line = Line::from(vec![Span::styled(
        format!(
            "  {} Ingress buffer synchronized · {start}–{end} tokens verified",
            agent_symbol("⎿", "\\")
        ),
        palette.muted(),
    )]);

    if target_height == 1 {
        return vec![Line::from(vec![
            Span::styled(format!("{spinner} "), palette.accent()),
            Span::styled(
                format!("Analyzing pipeline AST · stream_ingress [{start}..{end}] · active"),
                palette.body(),
            ),
        ])];
    }
    if target_height == 2 {
        return vec![
            Line::from(vec![
                Span::styled(format!("{spinner} "), palette.accent()),
                Span::styled(
                    format!(
                        "Running syntax tree analyzer... checking module section {current_section}"
                    ),
                    palette.accent(),
                ),
            ]),
            Line::from(vec![
                Span::styled(format!("{} ", agent_symbol("●", "*")), palette.accent()),
                Span::styled(
                    format!(
                        r#"inspect_buffer(channel="stream_ingress", offset={start}, limit={end}) · ready"#
                    ),
                    palette.body(),
                ),
            ]),
        ];
    }
    if target_height == 3 {
        return vec![
            Line::from(vec![
                Span::styled(format!("{spinner} "), palette.accent()),
                Span::styled(
                    format!("Running syntax tree analyzer... analyzing section {current_section}"),
                    palette.accent(),
                ),
            ]),
            tool_call_line,
            tool_res_line,
        ];
    }

    let workspace = state.config.agent_workspace();
    let target_file = state.config.agent_target();
    let density_label = match state.agent.density {
        AgentDensity::Comfortable => "comfortable",
        AgentDensity::Inspect => "inspect",
    };

    let mut pool: Vec<Line<'static>> = vec![
        inspect_line,
        Line::from(vec![
            Span::styled(format!("{spinner} "), palette.accent()),
            Span::styled(
                format!("Running syntax tree analyzer... checking AST for module {current_section}/{section_count}"),
                palette.accent(),
            ),
        ]),
        Line::from(vec![
            Span::styled(format!("  {} Running command -- ", agent_symbol("❯", ">")), palette.muted()),
            Span::styled("cargo clippy --workspace --all-targets -- -D warnings", palette.body()),
        ]),
        Line::styled(format!("  {tip}"), palette.muted()),
        Line::from(vec![
            Span::styled(format!("{} ", agent_symbol("●", "*")), palette.accent()),
            Span::styled("Context initialized", palette.body()),
        ]),
        Line::styled(
            format!("  workspace: {workspace} · session: local · target: {target_file}"),
            palette.muted(),
        ),
        tool_call_line,
        tool_res_line,
        Line::styled(
            format!(
                "  {} Runtime: async-tokio · Modules: {section_count} · Encoding: UTF-8",
                agent_symbol("⎿", "\\")
            ),
            palette.muted(),
        ),
        Line::from(vec![
            Span::styled(format!("{} ", agent_symbol("●", "*")), palette.accent()),
            Span::styled(
                format!("Context updated · Waiting for continuation · density: {density_label}"),
                palette.muted(),
            ),
        ]),
        Line::styled("  ✓ Unicode Standard Annex #14 line-breaking engine active (Unicode 15.0.0)", palette.muted()),
        Line::from(vec![
            Span::styled(format!("  {} ", agent_symbol("●", "*")), palette.accent()),
            Span::styled(r#"parse_syntax_tree(scope="pipeline::stream", depth=3)"#, palette.body()),
        ]),
        Line::styled(format!("    {} AST index warm · 142 syntax nodes active [OK]", agent_symbol("⎿", "\\")), palette.muted()),
        Line::from(vec![
            Span::styled(format!("  {} ", agent_symbol("●", "*")), palette.accent()),
            Span::styled(r#"check_type_signatures(interface="StreamProcessor", strict=true)"#, palette.body()),
        ]),
        Line::styled(format!("    {} Type signatures matched · trait bounds satisfied [OK]", agent_symbol("⎿", "\\")), palette.muted()),
        Line::from(vec![
            Span::styled(format!("  {} ", agent_symbol("●", "*")), palette.accent()),
            Span::styled(r#"verify_async_runtime(runtime="tokio", worker_threads=8)"#, palette.body()),
        ]),
        Line::styled(format!("    {} Task queue healthy · channel buffer occupancy: 4.2% [OK]", agent_symbol("⎿", "\\")), palette.muted()),
        Line::styled("  Memory: heap cache warm · 18,420 allocations indexed", palette.muted()),
        Line::styled("  Worker: async persistence channel idle (0 queued)", palette.muted()),
        Line::styled("  Snapshot: transaction committed · durable barrier synced", palette.muted()),
        Line::from(vec![
            Span::styled(format!("  {} ", agent_symbol("●", "*")), palette.accent()),
            Span::styled(r#"validate_schema_proto(package="telemetry.v1", syntax="proto3")"#, palette.body()),
        ]),
        Line::styled(format!("    {} Protobuf message definitions verified · wire format valid [OK]", agent_symbol("⎿", "\\")), palette.muted()),
        Line::from(vec![
            Span::styled(format!("  {} ", agent_symbol("●", "*")), palette.accent()),
            Span::styled(r#"inspect_connection_pool(backend="postgres", pool_size=16)"#, palette.body()),
        ]),
        Line::styled(format!("    {} Connection pool warm · query health check ping: 0.3ms [OK]", agent_symbol("⎿", "\\")), palette.muted()),
        Line::from(vec![
            Span::styled(format!("  {} ", agent_symbol("●", "*")), palette.accent()),
            Span::styled(r#"profile_memory_pool(allocator="mimalloc", max_heap="512MB")"#, palette.body()),
        ]),
        Line::styled(format!("    {} Active heap: 34.6 MB · zero fragmentation detected [OK]", agent_symbol("⎿", "\\")), palette.muted()),
        Line::from(vec![
            Span::styled(format!("  {} ", agent_symbol("●", "*")), palette.accent()),
            Span::styled(r#"run_bench_probe(target="batch_ingest", iterations=1000)"#, palette.body()),
        ]),
        Line::styled(format!("    {} Ingestion throughput: 284,000 events/sec [OK]", agent_symbol("⎿", "\\")), palette.muted()),
    ];

    if target_height <= pool.len() {
        pool.truncate(target_height);
        pool
    } else {
        let mut lines = pool;
        let mut idx = 1;
        while lines.len() < target_height {
            lines.push(Line::from(vec![
                Span::styled(format!("  {} ", agent_symbol("●", "*")), palette.accent()),
                Span::styled(
                    format!(
                        r#"sync_buffer_block(block_id={idx}, status="verified", hash="0x{:x}")"#,
                        0x8f2a_usize.wrapping_add(idx * 0x37)
                    ),
                    palette.muted(),
                ),
            ]));
            if lines.len() < target_height {
                lines.push(Line::styled(
                    format!(
                        "    {} Verified pipeline node consistency for block #{idx} [OK]",
                        agent_symbol("⎿", "\\")
                    ),
                    palette.muted(),
                ));
            }
            idx += 1;
        }
        lines.truncate(target_height);
        lines
    }
}

fn draw_discreet(frame: &mut ratatui::Frame<'_>, state: &ReaderState, palette: Palette) {
    let layout = AgentLayout::with_options(
        frame.area(),
        state.config.text_width,
        state.config.agent_lines,
    );
    if frame.area().height < 6 || layout.body.width == 0 {
        frame.render_widget(
            Paragraph::new("Resize terminal").style(palette.muted()),
            frame.area(),
        );
        return;
    }
    let (start, end, total) = state.physical_row_range();
    let net_height = layout.net_text_height();
    let width = frame.area().width;

    // 1. Top Session Context Banner (Full mode only)
    if layout.mode == AgentViewMode::Full && layout.banner.height >= 3 {
        let banner_lines = vec![
            split_row_line(
                vec![
                    Span::styled("workspace: ", palette.muted()),
                    Span::styled(state.config.agent_workspace().to_string(), palette.body()),
                ],
                vec![
                    Span::styled("session: ", palette.muted()),
                    Span::styled("local", palette.body()),
                ],
                width,
            ),
            split_row_line(
                vec![
                    Span::styled("task: ", palette.muted()),
                    Span::styled(state.config.agent_task().to_string(), palette.body()),
                ],
                vec![
                    Span::styled("target: ", palette.muted()),
                    Span::styled(state.config.agent_target().to_string(), palette.body()),
                ],
                width,
            ),
            Line::styled(
                agent_symbol("─", "-").repeat(usize::from(width)),
                palette.muted(),
            ),
        ];
        frame.render_widget(Paragraph::new(banner_lines), layout.banner);
    }

    if state.hidden {
        let middle_height = frame
            .area()
            .height
            .saturating_sub(layout.banner.height + layout.footer.height);
        let middle_area = Rect::new(
            frame.area().x,
            layout.banner.bottom(),
            frame.area().width,
            middle_height,
        );
        let tool_lines = simulated_tool_lines(usize::from(middle_area.height), palette);
        frame.render_widget(Paragraph::new(tool_lines), middle_area);

        let prompt_spans = vec![
            Span::styled(format!("{} ", agent_symbol("❯", ">")), palette.muted()),
            Span::styled("continue_", palette.muted()),
        ];
        let footer_lines = if layout.mode == AgentViewMode::Full && layout.footer.height >= 3 {
            vec![
                Line::styled(
                    agent_symbol("─", "-").repeat(usize::from(width)),
                    palette.muted(),
                ),
                split_row_line(
                    vec![
                        Span::styled("activity: ", palette.muted()),
                        Span::styled("cargo-test ✓ · exit code 0", palette.accent()),
                    ],
                    vec![Span::styled("38 passed · Output folded", palette.muted())],
                    width,
                ),
                split_row_line(
                    prompt_spans,
                    vec![Span::styled("F12 resume", palette.muted())],
                    width,
                ),
            ]
        } else {
            match layout.footer.height {
                0 => Vec::new(),
                1 => vec![Line::from(prompt_spans)],
                _ => vec![
                    Line::from(prompt_spans),
                    Line::styled(
                        "  cargo-test ✓ · 38 passed · Output folded · F12 resume",
                        palette.muted(),
                    ),
                ],
            }
        };
        frame.render_widget(Paragraph::new(footer_lines), layout.footer);
        return;
    }

    // 2. Session Context & Tool Activity (header)
    let header_lines = generate_agent_header_lines(
        usize::from(layout.header.height),
        state,
        palette,
        start,
        end,
    );
    frame.render_widget(Paragraph::new(header_lines), layout.header);

    // 3. Assistant Response (body)
    let mut body_lines = Vec::new();
    if state.agent.help {
        let mut help_entries = vec![
            Line::from("Local commands · j/k scroll · Esc close"),
            Line::from(""),
        ];
        help_entries.extend(
            DISCREET_BINDINGS
                .iter()
                .map(|b| Line::from(format!("  {:<12} {}", b.keys, b.label))),
        );
        help_entries.push(Line::from(""));
        help_entries.push(Line::from("Executable local commands:"));
        help_entries.extend(
            LOCAL_COMMANDS
                .iter()
                .map(|(name, _)| Line::from(format!("  :{:<11} execute command", name))),
        );
        let visible_help = help_entries
            .into_iter()
            .skip(state.agent.help_offset)
            .take(usize::from(net_height));
        for line in visible_help {
            body_lines.push(Line::from(vec![
                Span::styled("  ", palette.muted()),
                Span::styled(line.to_string(), palette.body()),
            ]));
        }
    } else {
        body_lines.push(Line::from(vec![
            Span::styled(format!("{} ", agent_symbol("●", "*")), palette.accent()),
            Span::styled("Assistant", palette.body()),
        ]));

        let visible_lines = state.agent.view.visible(net_height);
        for (i, vline) in visible_lines.iter().enumerate() {
            let line_num = start + i;
            let is_overlap = i < state.agent.view.context_rows;
            let text_style = if is_overlap {
                palette.muted()
            } else {
                palette.body()
            };
            let raw_text = &vline.line.text;
            match state.agent.density {
                AgentDensity::Comfortable => {
                    body_lines.push(Line::from(vec![Span::styled(raw_text.clone(), text_style)]));
                }
                AgentDensity::Inspect => {
                    body_lines.push(Line::from(vec![
                        Span::styled(format!("{:>4} │ ", line_num), palette.muted()),
                        Span::styled(raw_text.clone(), text_style),
                    ]));
                }
            }
        }

        if layout.body.height > net_height + 1 {
            body_lines.push(Line::from(vec![Span::styled(
                "  [Stream buffer synchronized · 0 warnings · ready]",
                palette.muted(),
            )]));
        }
    }
    frame.render_widget(Paragraph::new(body_lines), layout.body);

    // 4. Activity & Actions (footer)
    let current_chunk = (start / net_height as usize).saturating_add(1);
    let total_chunks = (total / net_height as usize).max(1);
    let pct = (start * 100 / total.max(1)).min(100);
    let stream_pace = if state.auto_reading.active {
        let cpm = state.auto_reading.cpm;
        if state.auto_reading.paused {
            Some(format!("stream: {cpm} cpm [paused]"))
        } else {
            let rem_secs = state.auto_reading.remaining_time(Instant::now()).as_secs();
            match state.auto_reading.mode {
                crate::auto_reading::AutoReadingMode::TimedPage => {
                    Some(format!("stream: {cpm} cpm ({rem_secs}s)"))
                }
                crate::auto_reading::AutoReadingMode::LineScroll => {
                    Some(format!("stream: {cpm} cpm [scroll]"))
                }
            }
        }
    } else {
        None
    };

    let chunk_label = match &stream_pace {
        Some(pace) => format!("batch {current_chunk}/{total_chunks} ({pct}%) · {pace}"),
        None => format!("batch {current_chunk}/{total_chunks} ({pct}%)"),
    };

    let status_hint = match state.agent.receipt.as_deref() {
        Some(receipt) => format!("✓ {receipt}"),
        None if state.agent.command.is_some() => {
            "Enter execute · Esc cancel · Tab complete".to_string()
        }
        None if state.agent.help => "j/k scroll · Esc close".to_string(),
        None => match stream_pace {
            Some(pace) => format!("{pace} · :help"),
            None => ":help · :density · :checkpoint · :fold".to_string(),
        },
    };

    let prompt_spans = match &state.agent.command {
        Some(text) => vec![
            Span::styled(format!("{} :", agent_symbol("❯", ">")), palette.accent()),
            Span::styled(text.clone(), palette.body()),
            Span::styled("█", palette.accent()),
        ],
        None => vec![
            Span::styled(format!("{} ", agent_symbol("❯", ">")), palette.muted()),
            Span::styled("continue_", palette.muted()),
        ],
    };

    let footer_lines = if layout.mode == AgentViewMode::Full && layout.footer.height >= 3 {
        vec![
            Line::styled(
                agent_symbol("─", "-").repeat(usize::from(width)),
                palette.muted(),
            ),
            split_row_line(
                vec![
                    Span::styled("activity: ", palette.muted()),
                    Span::styled("parse ✓   validate ✓   sync ✓", palette.accent()),
                ],
                vec![Span::styled(chunk_label, palette.muted())],
                width,
            ),
            split_row_line(
                prompt_spans,
                vec![Span::styled(status_hint, palette.muted())],
                width,
            ),
        ]
    } else {
        match layout.footer.height {
            0 => Vec::new(),
            1 => vec![Line::from(prompt_spans)],
            _ => vec![
                Line::from(prompt_spans),
                Line::styled(format!("  {status_hint}"), palette.muted()),
            ],
        }
    };
    frame.render_widget(Paragraph::new(footer_lines), layout.footer);

    if let Some(text) = state.agent.command.as_ref() {
        let prompt_row_offset = if layout.mode == AgentViewMode::Full && layout.footer.height >= 3 {
            2
        } else {
            0
        };
        let row = layout.footer.y + prompt_row_offset;
        let col = (3 + text.width() as u16).min(layout.footer.width.saturating_sub(1));
        frame.set_cursor_position((layout.footer.x + col, row));
    }
}

fn simulated_tool_lines(target_height: usize, palette: Palette) -> Vec<Line<'static>> {
    if target_height == 0 {
        return Vec::new();
    }

    let inspect_line = Line::from(vec![
        Span::styled(format!("{} ", agent_symbol("❯", ">")), palette.accent()),
        Span::styled("cargo test --workspace --lib", palette.body()),
    ]);
    let tool_call_line = Line::from(vec![
        Span::styled(format!("{} ", agent_symbol("●", "*")), palette.accent()),
        Span::styled(
            r#"run_command command="cargo test --workspace --lib""#,
            palette.body(),
        ),
        Span::styled(" · Output folded", palette.muted()),
    ]);
    let success_exit_line = Line::styled(
        "  [Tool execution completed successfully · exit code 0]",
        palette.muted(),
    );

    if target_height == 1 {
        return vec![tool_call_line];
    }
    if target_height == 2 {
        return vec![inspect_line, tool_call_line];
    }
    if target_height == 3 {
        return vec![inspect_line, tool_call_line, success_exit_line];
    }

    let compile_lines = [
        Line::styled(
            "   Compiling core-runtime v0.4.2 (/workspace/data-mesh-service)",
            palette.muted(),
        ),
        Line::styled("   Checking tokio v1.38.0", palette.muted()),
        Line::styled("   Checking prost v0.13.1", palette.muted()),
        Line::styled(
            "    Finished `test` profile [unoptimized + debuginfo] in 0.85s",
            palette.muted(),
        ),
        Line::styled(
            "     Running unittests src/lib.rs (target/debug/deps/core_runtime-d1838cf4023)",
            palette.muted(),
        ),
    ];

    if target_height < 12 {
        let mut lines = Vec::new();
        lines.push(inspect_line);
        lines.push(tool_call_line);
        for cl in &compile_lines {
            if lines.len() + 1 >= target_height {
                break;
            }
            lines.push(cl.clone());
        }
        if lines.len() < target_height {
            lines.push(success_exit_line);
        }
        lines.truncate(target_height);
        return lines;
    }

    let header_block = vec![
        inspect_line,
        tool_call_line,
        compile_lines[0].clone(),
        compile_lines[1].clone(),
        compile_lines[2].clone(),
        compile_lines[3].clone(),
        compile_lines[4].clone(),
        Line::from(""),
        Line::styled("running 38 tests", palette.body()),
    ];

    let summary_block = vec![
        Line::from(""),
        Line::styled(
            "test result: ok. 38 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out",
            palette.accent(),
        ),
        success_exit_line,
    ];

    let test_cases = [
        "test pipeline::tests::test_stream_ingress ... ok",
        "test storage::tests::test_partition_dedup ... ok",
        "test buffer::tests::test_ring_buffer_capacity ... ok",
        "test telemetry::tests::test_counter_metrics ... ok",
        "test parser::tests::test_proto_wire_decode ... ok",
        "test transport::tests::test_grpc_streaming ... ok",
        "test config::tests::test_cluster_endpoints ... ok",
        "test atomic_file::tests::test_durable_wal_sync ... ok",
        "test cluster::tests::test_leader_election ... ok",
        "test schema::tests::test_registry_evolution ... ok",
        "test network::tests::test_backpressure_threshold ... ok",
        "test journal::tests::test_wal_checkpoint_replay ... ok",
        "test parser::tests::test_utf8_payload_validation ... ok",
        "test transform::tests::test_windowed_aggregation ... ok",
        "test query::tests::test_predicate_pushdown ... ok",
        "test session::tests::test_client_heartbeat ... ok",
        "test codec::tests::test_snappy_compression ... ok",
        "test pool::tests::test_connection_eviction ... ok",
        "test security::tests::test_token_signature_verify ... ok",
        "test cache::tests::test_lru_invalidation ... ok",
        "test scheduler::tests::test_priority_queue ... ok",
        "test router::tests::test_load_balance_weighted ... ok",
        "test transform::tests::test_cjk_token_boundaries ... ok",
        "test storage::tests::test_snapshot_parity ... ok",
        "test query::tests::test_index_scan_range ... ok",
        "test network::tests::test_circuit_breaker_trip ... ok",
        "test buffer::tests::test_overflow_spill_to_disk ... ok",
        "test session::tests::test_throughput_watermark ... ok",
        "test audit::tests::test_immutable_event_log ... ok",
        "test config::tests::test_dynamic_reload ... ok",
        "test codec::tests::test_varint_encoding ... ok",
        "test journal::tests::test_segment_compaction ... ok",
        "test parser::tests::test_delimiter_fuzzing ... ok",
        "test transport::tests::test_tls_handshake_timeout ... ok",
        "test router::tests::test_routing_table_diff ... ok",
        "test cluster::tests::test_split_brain_recovery ... ok",
        "test storage::tests::test_atomic_commit_log ... ok",
        "test telemetry::tests::test_histogram_latency_percentiles ... ok",
    ];

    if target_height <= 50 {
        let available_tests = target_height - 12;
        let mut lines = header_block;
        for tc in &test_cases[..available_tests] {
            lines.push(Line::styled(format!("  {tc}"), palette.muted()));
        }
        lines.extend(summary_block);
        return lines;
    }

    let doctest_block = vec![
        Line::from(""),
        Line::styled("   Doc-tests core_runtime", palette.muted()),
        Line::from(""),
        Line::styled("running 0 tests", palette.body()),
        Line::from(""),
        Line::styled(
            "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out",
            palette.accent(),
        ),
    ];

    if target_height <= 56 {
        let available_doctests = target_height - 50;
        let mut lines = header_block;
        for tc in &test_cases {
            lines.push(Line::styled(format!("  {tc}"), palette.muted()));
        }
        lines.extend(summary_block);
        lines.extend(doctest_block.into_iter().take(available_doctests));
        return lines;
    }

    // target_height > 56: prepend authentic prior session history
    let extra_needed = target_height - 56;
    let prior_history = [
        Line::from(vec![
            Span::styled(format!("{} ", agent_symbol("❯", ">")), palette.accent()),
            Span::styled("git status --short", palette.body()),
        ]),
        Line::styled(" M crates/core/src/pipeline.rs", palette.muted()),
        Line::styled(" M Cargo.lock", palette.muted()),
        Line::styled("?? target/", palette.muted()),
        Line::from(""),
        Line::from(vec![
            Span::styled(format!("{} ", agent_symbol("❯", ">")), palette.accent()),
            Span::styled(
                "cargo clippy --workspace --all-targets -- -D warnings",
                palette.body(),
            ),
        ]),
        Line::styled(
            "   Compiling core-runtime v0.4.2 (/workspace/data-mesh-service)",
            palette.muted(),
        ),
        Line::styled("    Checking tokio v1.38.0", palette.muted()),
        Line::styled(
            "    Finished `dev` profile [unoptimized + debuginfo] in 0.58s",
            palette.muted(),
        ),
        Line::styled(
            "  [Clippy check completed successfully · exit code 0]",
            palette.muted(),
        ),
        Line::from(""),
        Line::from(vec![
            Span::styled(format!("{} ", agent_symbol("❯", ">")), palette.accent()),
            Span::styled("cargo check --tests", palette.body()),
        ]),
        Line::styled(
            "    Checking core-runtime v0.4.2 (/workspace/data-mesh-service)",
            palette.muted(),
        ),
        Line::styled(
            "    Finished `test` profile [unoptimized + debuginfo] in 0.42s",
            palette.muted(),
        ),
        Line::styled(
            "  [Cargo check passed · 0 warnings · 0 errors]",
            palette.muted(),
        ),
        Line::from(""),
        Line::from(vec![
            Span::styled(format!("{} ", agent_symbol("❯", ">")), palette.accent()),
            Span::styled("rustc --version", palette.body()),
        ]),
        Line::styled("rustc 1.82.0 (f6e511eec 2024-10-15)", palette.muted()),
        Line::from(""),
    ];

    let mut full_test_run = header_block;
    for tc in &test_cases {
        full_test_run.push(Line::styled(format!("  {tc}"), palette.muted()));
    }
    full_test_run.extend(summary_block);
    full_test_run.extend(doctest_block);

    let mut prefix = Vec::new();
    while prefix.len() < extra_needed {
        let remaining = extra_needed - prefix.len();
        if remaining >= prior_history.len() {
            prefix.extend(prior_history.iter().cloned());
        } else {
            prefix.extend(
                prior_history[prior_history.len() - remaining..]
                    .iter()
                    .cloned(),
            );
        }
    }
    prefix.extend(full_test_run);
    prefix
}

fn truncate_width(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    if width <= 1 {
        return "…".to_string();
    }
    let mut out = String::new();
    for ch in text.chars() {
        let candidate = format!("{out}{ch}…");
        if candidate.width() > width {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::book::{
        BookDocument, BookFormat, BookMetadata, ContentsEntry, ContentsSource, Section,
        TextPosition,
    };
    use crate::config::ReaderConfig;
    use crate::storage::Progress;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn state() -> ReaderState {
        let path = std::path::PathBuf::from("book.txt");
        let sections = vec![Section {
            title: "Opening".into(),
            text: "bright readable text".into(),
        }];
        ReaderState::new(
            BookDocument {
                id: "book".into(),
                title: "Book".into(),
                path: path.clone(),
                format: BookFormat::Txt,
                metadata: BookMetadata {
                    title: "Book".into(),
                    author: None,
                    language: None,
                    publisher: None,
                    format: BookFormat::Txt,
                    path,
                    section_count: 1,
                },
                sections,
                contents: vec![ContentsEntry {
                    title: "Opening".into(),
                    depth: 0,
                    target: crate::book::TextPosition::new(0, 0),
                }],
                contents_source: ContentsSource::Generated,
            },
            "book".into(),
            Rect::new(0, 0, 80, 12),
            ReaderConfig::default(),
            Progress::default(),
            Vec::new(),
        )
    }

    fn pdf_state() -> ReaderState {
        let path = std::path::PathBuf::from("manual.pdf");
        let sections = vec![
            Section {
                title: "Page 1".into(),
                text: "first PDF page".into(),
            },
            Section {
                title: "Page 2".into(),
                text: "second PDF page".into(),
            },
        ];
        ReaderState::new(
            BookDocument {
                id: "pdf".into(),
                title: "Manual".into(),
                path: path.clone(),
                format: BookFormat::Pdf,
                metadata: BookMetadata {
                    title: "Manual".into(),
                    author: Some("Writer".into()),
                    language: None,
                    publisher: None,
                    format: BookFormat::Pdf,
                    path,
                    section_count: sections.len(),
                },
                sections,
                contents: vec![
                    ContentsEntry {
                        title: "Introduction".into(),
                        depth: 0,
                        target: crate::book::TextPosition::new(0, 0),
                    },
                    ContentsEntry {
                        title: "Nested topic".into(),
                        depth: 1,
                        target: crate::book::TextPosition::new(1, 0),
                    },
                ],
                contents_source: ContentsSource::Document,
            },
            "pdf".into(),
            Rect::new(0, 0, 80, 20),
            ReaderConfig::default(),
            Progress::default(),
            Vec::new(),
        )
    }

    #[test]
    fn reading_view_inherits_terminal_foreground_and_background() {
        let state = state();
        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &state)).unwrap();
        let cell = terminal.backend().buffer().cell((0, 1)).unwrap();
        assert_eq!(cell.fg, ratatui::style::Color::Reset);
        assert_eq!(cell.bg, ratatui::style::Color::Reset);
    }

    #[test]
    fn narrow_hud_keeps_core_keys() {
        let line = crate::ui::action_line(READING_BINDINGS, 20, Palette::terminal());
        let text = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        for key in ["k", "j", "g", "t", "?", "q"] {
            assert!(text.contains(key));
        }
        assert!(text.width() <= 20);
    }

    #[test]
    fn every_reader_overlay_renders_with_its_context() {
        let cases = [
            (Overlay::Percent(Default::default()), "Go to percentage"),
            (Overlay::Contents(Default::default()), "Contents"),
            (Overlay::Search(Default::default()), "Search"),
            (Overlay::Metadata, "Book information"),
            (Overlay::Help, "Reader help"),
        ];

        for (overlay, expected) in cases {
            let mut state = state();
            state.overlay = overlay;
            let backend = TestBackend::new(80, 20);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal.draw(|frame| draw(frame, &state)).unwrap();
            assert!(format!("{}", terminal.backend()).contains(expected));
        }
    }

    #[test]
    fn generated_contents_are_explicitly_labeled() {
        let mut state = state();
        state.overlay = Overlay::Contents(Default::default());
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| draw(frame, &state)).unwrap();

        let screen = format!("{}", terminal.backend());
        assert!(screen.contains("Contents (generated)"));
    }

    #[test]
    fn pdf_reader_uses_page_and_contents_labels() {
        let mut state = pdf_state();
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| draw(frame, &state)).unwrap();
        let screen = format!("{}", terminal.backend());
        assert!(screen.contains("Page 1/2: Page 1"));

        state.overlay = Overlay::Contents(Default::default());
        terminal.draw(|frame| draw(frame, &state)).unwrap();
        let screen = format!("{}", terminal.backend());
        assert!(screen.contains("Contents"));
        assert!(screen.contains("Entry number:"));
        assert!(screen.contains("2 matching entries"));
        assert!(screen.contains("Nested topic"));
        assert!(screen.contains("    2    Nested topic"));

        state.overlay = Overlay::Metadata;
        terminal.draw(|frame| draw(frame, &state)).unwrap();
        let screen = format!("{}", terminal.backend());
        assert!(screen.contains("Format: PDF"));
        assert!(screen.contains("Pages: 2"));
    }

    #[test]
    fn agent_terminal_renders_real_content_without_metadata_or_logs() {
        let mut s = state();
        s.resize(80, 24);
        s.update(crate::reader_state::ReaderAction::ToggleDiscreet);
        s.notify("SECRET notification", NotificationKind::Error);
        s.overlay = Overlay::Metadata;
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| draw(f, &s)).unwrap();
        let screen = format!("{}", terminal.backend());
        for expected in [
            "inspect_buffer",
            "Assistant",
            "Ingress buffer synchronized",
            "bright readable text",
            "continue_",
        ] {
            assert!(screen.contains(expected), "{screen}");
        }
        for secret in [
            "SECRET",
            "TypeError",
            "INFO",
            "WARN",
            "Book information",
            "book.txt",
            "Reader-cli",
            "read_chunk",
        ] {
            assert!(!screen.contains(secret), "Leaked secret: {secret}");
        }
        let first = terminal.backend().buffer().clone();
        terminal.draw(|f| draw(f, &s)).unwrap();
        assert_eq!(&first, terminal.backend().buffer());
    }

    #[test]
    fn agent_terminal_four_semantic_regions_and_comfortable_rows() {
        for (w, h, expected_net_rows, is_full) in
            [(80, 24, 7, true), (60, 18, 5, false), (120, 30, 9, true)]
        {
            let mut s = state();
            s.resize(w, h);
            s.update(crate::reader_state::ReaderAction::ToggleDiscreet);
            let layout = AgentLayout::new(s.viewport, s.config.text_width);
            assert_eq!(layout.net_text_height(), expected_net_rows);
            // 60:40 verification: prose <= 40% of height, Agent >= 50%
            assert!(f64::from(layout.net_text_height()) / f64::from(h) <= 0.40);
            assert!(
                f64::from(layout.banner.height + layout.header.height + layout.footer.height)
                    / f64::from(h)
                    >= 0.50
            );

            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal.draw(|f| draw(f, &s)).unwrap();
            let screen = format!("{}", terminal.backend());

            if is_full {
                // Top Banner & Scheme A context
                assert!(screen.contains("workspace: data-mesh-service"));
                assert!(screen.contains("session: local"));
                assert!(screen.contains("task: Code Review & Refactor"));
                assert!(screen.contains("target: crates/core/src/pipeline.rs"));
                assert!(screen.contains("Context initialized"));
                assert!(screen.contains("activity: parse ✓   validate ✓   sync ✓"));
                assert!(screen.contains("batch "));
            }

            // Region 1 & 2: Session Context and Tool Activity
            assert!(screen.contains("inspect_buffer"));
            assert!(screen.contains("Ingress buffer synchronized"));

            // Region 3: Assistant Response with verbatim prose
            assert!(screen.contains("Assistant"));
            assert!(screen.contains("bright readable text"));
            assert!(screen.contains("[Stream buffer synchronized · 0 warnings · ready]"));

            // Region 4: Input / Action
            assert!(screen.contains("continue_"));
        }
    }

    #[test]
    fn high_terminal_mode_populates_full_agent_lines_without_empty_space() {
        // High full terminal (120x45)
        let mut s = state();
        s.resize(120, 45);
        s.update(crate::reader_state::ReaderAction::ToggleDiscreet);
        let layout = AgentLayout::new(s.viewport, s.config.text_width);
        assert_eq!(layout.header.height, 25);
        let mut terminal = Terminal::new(TestBackend::new(120, 45)).unwrap();
        terminal.draw(|f| draw(f, &s)).unwrap();
        let screen = format!("{}", terminal.backend());

        // Header has 25 lines full of realistic Agent trace steps
        assert!(screen.contains("workspace: data-mesh-service"));
        assert!(screen.contains("optimize stream ingestion and telemetry buffer pipeline"));
        assert!(screen.contains("cargo clippy --workspace --all-targets -- -D warnings"));
        assert!(screen.contains(r#"inspect_buffer(channel="stream_ingress""#));
        assert!(screen.contains(r#"parse_syntax_tree(scope="pipeline::stream""#));
        assert!(screen.contains(r#"check_type_signatures(interface="StreamProcessor""#));
        assert!(screen.contains(r#"verify_async_runtime(runtime="tokio""#));
        assert!(screen.contains("Ingress buffer synchronized"));
        assert!(screen.contains("Context updated · Waiting for continuation"));

        // High compact terminal (60x40) with custom agent lines = 18
        s.resize(60, 40);
        s.update(crate::reader_state::ReaderAction::SetAgentLines(Some(18)));
        let compact_layout =
            AgentLayout::with_options(s.viewport, s.config.text_width, s.config.agent_lines);
        assert_eq!(compact_layout.header.height, 18);
        let mut terminal_compact = Terminal::new(TestBackend::new(60, 40)).unwrap();
        terminal_compact.draw(|f| draw(f, &s)).unwrap();
        let compact_screen = format!("{}", terminal_compact.backend());
        assert!(compact_screen.contains("optimize stream ingestion"));
        assert!(compact_screen.contains(r#"inspect_buffer(channel="stream_ingress""#));
        assert!(compact_screen.contains(r#"parse_syntax_tree"#));
        assert!(compact_screen.contains("Ingress buffer synchronized"));
    }

    #[test]
    fn agent_result_preserves_blank_rows_indent_and_terminal_colors() {
        let mut s = state();
        s.document.sections[0].text = "  indented line\n\n  next paragraph".into();
        s.resize(80, 24);
        s.update(crate::reader_state::ReaderAction::ToggleDiscreet);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| draw(f, &s)).unwrap();
        let area = AgentLayout::new(s.viewport, s.config.text_width).body;
        let buffer = terminal.backend().buffer();
        // Comfortable mode (default): Assistant banner on row 0, content starts row 1
        // Row 1: 2-space original indent puts 'i' at area.x + 2
        assert_eq!(buffer.cell((area.x + 2, area.y + 1)).unwrap().symbol(), "i");
        // Row 2 is blank line; Row 3 is next paragraph
        assert_eq!(buffer.cell((area.x + 2, area.y + 3)).unwrap().symbol(), "n");
        let body = buffer.cell((area.x + 2, area.y + 1)).unwrap();
        assert_eq!(body.fg, ratatui::style::Color::Reset);
        assert_eq!(body.bg, ratatui::style::Color::Reset);
        assert!(!body.modifier.contains(ratatui::style::Modifier::DIM));

        // Inspect mode: toggle density to show line numbers
        s.update(crate::reader_state::ReaderAction::ToggleDensity);
        terminal.draw(|f| draw(f, &s)).unwrap();
        let inspect_buf = terminal.backend().buffer();
        // Line number 1 is "    1 │ " (7 columns) followed by 2 spaces indent, 'i' is at area.x + 7 + 2 = 9
        assert_eq!(
            inspect_buf.cell((area.x + 9, area.y + 1)).unwrap().symbol(),
            "i"
        );
    }

    #[test]
    fn f12_clears_old_frames_in_every_approved_size_and_after_resize() {
        let mut s = state();
        s.document.title = "SECRET_TITLE".into();
        s.document.path = "C:/SECRET_PATH/book.txt".into();
        s.document.metadata.author = Some("SECRET_AUTHOR".into());
        s.document.sections[0].text = "SECRET_BODY ".repeat(1000);
        s.search_query = Some("SECRET_QUERY".into());
        s.notify("SECRET_NOTICE", NotificationKind::Error);
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        for (w, h) in [(80, 24), (120, 30), (60, 18), (3, 2)] {
            terminal.backend_mut().resize(w, h);
            terminal.resize(Rect::new(0, 0, w, h)).unwrap();
            s.resize(w, h);
            s.hidden = false;
            terminal.draw(|f| draw(f, &s)).unwrap();
            s.update(crate::reader_state::ReaderAction::ToggleHidden);
            terminal.draw(|f| draw(f, &s)).unwrap();
            let screen = format!("{}", terminal.backend());
            assert!(!screen.contains("SECRET"));
            assert!(!screen.contains("book.txt"));
            assert!(!screen.contains("TypeError"));
            if w > 30 {
                assert!(
                    screen.contains("output folded")
                        || screen.contains("Output folded")
                        || screen.contains("run_command")
                );
            }
        }
    }

    #[test]
    fn f12_subtle_in_place_fold_in_both_discreet_and_normal_modes() {
        // 1. Discreet mode test
        let mut s = state();
        s.resize(80, 24);
        s.update(crate::reader_state::ReaderAction::ToggleDiscreet);
        assert!(s.config.discreet_mode);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| draw(f, &s)).unwrap();
        let reading_screen = format!("{}", terminal.backend());
        assert!(reading_screen.contains("workspace: data-mesh-service"));
        assert!(reading_screen.contains("Code Review & Refactor"));
        assert!(reading_screen.contains("Assistant"));

        // Toggle hidden:
        s.update(crate::reader_state::ReaderAction::ToggleHidden);
        terminal.draw(|f| draw(f, &s)).unwrap();
        let folded_screen = format!("{}", terminal.backend());
        // Outer banner remains identical
        assert!(folded_screen.contains("workspace: data-mesh-service"));
        assert!(folded_screen.contains("Code Review & Refactor"));
        // Body was subtly replaced with tool execution
        assert!(folded_screen.contains("run_command command="));
        assert!(folded_screen.contains("Output folded"));
        assert!(folded_screen.contains("running 38 tests"));
        assert!(folded_screen.contains("F12 resume"));

        // Toggle back:
        s.update(crate::reader_state::ReaderAction::ToggleHidden);
        terminal.draw(|f| draw(f, &s)).unwrap();
        let resumed_screen = format!("{}", terminal.backend());
        assert!(resumed_screen.contains("Assistant"));
        assert!(resumed_screen.contains("bright readable text"));

        // 2. Normal mode test
        let mut s2 = state();
        s2.resize(80, 24);
        assert!(!s2.config.discreet_mode);
        terminal.draw(|f| draw(f, &s2)).unwrap();
        let normal_screen = format!("{}", terminal.backend());
        assert!(normal_screen.contains("《Book》"));

        // Toggle hidden in normal mode:
        s2.update(crate::reader_state::ReaderAction::ToggleHidden);
        terminal.draw(|f| draw(f, &s2)).unwrap();
        let normal_folded = format!("{}", terminal.backend());
        assert!(!normal_folded.contains("《Book》"));
        assert!(normal_folded.contains("cargo test"));
        assert!(normal_folded.contains("Output folded"));
        assert!(normal_folded.contains("running 38 tests"));
        assert!(normal_folded.contains("[F12 resume]"));
    }

    #[test]
    fn f12_in_tall_terminal_has_no_large_blank_gaps() {
        let mut s = state();
        s.resize(120, 60);
        s.update(crate::reader_state::ReaderAction::ToggleDiscreet);
        s.update(crate::reader_state::ReaderAction::ToggleHidden);
        assert!(s.hidden);

        let mut terminal = Terminal::new(TestBackend::new(120, 60)).unwrap();
        terminal.draw(|f| draw(f, &s)).unwrap();
        let screen = format!("{}", terminal.backend());

        assert!(screen.contains("workspace: data-mesh-service"));
        assert!(screen.contains("cargo test --workspace --lib"));
        assert!(screen.contains("running 38 tests"));
        assert!(screen.contains("test result: ok. 38 passed"));
        assert!(screen.contains("F12 resume"));

        // Verify that there are no large gaps of blank lines in the middle area (rows 3..57).
        let buffer = terminal.backend().buffer();
        let mut max_consecutive_empty = 0;
        let mut current_consecutive_empty = 0;
        for y in 3..57 {
            let row_empty = (0..120).all(|x| {
                buffer
                    .cell((x, y))
                    .is_none_or(|c| c.symbol().trim().is_empty())
            });
            if row_empty {
                current_consecutive_empty += 1;
                max_consecutive_empty = max_consecutive_empty.max(current_consecutive_empty);
            } else {
                current_consecutive_empty = 0;
            }
        }
        assert!(
            max_consecutive_empty <= 1,
            "Found excessive consecutive empty lines in tall terminal folded view: {max_consecutive_empty}"
        );
    }

    #[test]
    fn agent_checkpoint_receipt_never_moves_the_body() {
        let mut s = state();
        s.resize(80, 24);
        s.update(crate::reader_state::ReaderAction::ToggleDiscreet);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| draw(f, &s)).unwrap();
        let before = terminal.backend().buffer().clone();
        s.agent.receipt = Some("Checkpoint saved".into());
        s.agent.checkpoints = 1;
        terminal.draw(|f| draw(f, &s)).unwrap();
        let area = AgentLayout::new(s.viewport, s.config.text_width).body;
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                assert_eq!(
                    before.cell((x, y)),
                    terminal.backend().buffer().cell((x, y))
                );
            }
        }
    }

    #[test]
    fn search_highlighting_distinguishes_current_and_secondary_matches_and_handles_multiline() {
        let mut state = state();
        state.search_query = Some("bright".into());
        // "bright readable text" 中 "bright" 在开头 0..6
        state.search_results = vec![
            crate::book::SearchResult {
                position: TextPosition::new(0, 0),
                end_position: TextPosition::new(0, 6),
                preview: "bright readable text".into(),
            },
            crate::book::SearchResult {
                position: TextPosition::new(0, 7),
                end_position: TextPosition::new(0, 15),
                preview: "readable".into(),
            },
        ];
        state.search_index = 0;

        let palette = Palette::terminal();
        let rendered = render_reading_lines(&state, palette);
        let has_current = rendered.iter().any(|line| {
            line.spans
                .iter()
                .any(|s| s.style == palette.search_match_current() && s.content == "bright")
        });
        assert!(
            has_current,
            "Should highlight current match with current style"
        );

        state.search_index = 1;
        let rendered_next = render_reading_lines(&state, palette);
        let has_current_next = rendered_next.iter().any(|line| {
            line.spans
                .iter()
                .any(|s| s.style == palette.search_match_current() && s.content == "readable")
        });
        assert!(
            has_current_next,
            "Should highlight match 1 as current after switching index"
        );
    }

    #[test]
    fn search_results_overlay_renders_list_with_selected_row() {
        let mut state = state();
        state.search_query = Some("test".into());
        state.search_results = vec![
            crate::book::SearchResult {
                position: TextPosition::new(0, 0),
                end_position: TextPosition::new(0, 4),
                preview: "test match 1".into(),
            },
            crate::book::SearchResult {
                position: TextPosition::new(0, 20),
                end_position: TextPosition::new(0, 24),
                preview: "test match 2".into(),
            },
        ];
        state.overlay = Overlay::SearchResults(crate::reader_state::SearchResultsNavigator {
            selection: 1,
            error: None,
        });

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &state)).unwrap();
        let screen = format!("{}", terminal.backend());
        assert!(screen.contains("Search Results"));
        assert!(screen.contains("test match 1"));
        assert!(screen.contains("test match 2"));
        assert!(screen.contains("2 matching results"));
    }

    #[test]
    fn immersive_mode_hides_status_and_action_bars() {
        let mut state = state();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        // Normal mode renders title and footer progress bar
        terminal.draw(|frame| draw(frame, &state)).unwrap();
        let normal_screen = format!("{}", terminal.backend());
        assert!(normal_screen.contains("Book"));
        assert!(normal_screen.contains("Section") || normal_screen.contains("Sec"));

        // Immersive mode hides title and footer
        state.config.immersive_mode = true;
        terminal.draw(|frame| draw(frame, &state)).unwrap();
        let immersive_screen = format!("{}", terminal.backend());
        assert!(!immersive_screen.contains("《Book》"));
        assert!(!immersive_screen.contains("Section"));
    }

    #[test]
    fn bookmarks_overlay_renders_list_and_handles_empty() {
        let mut state = state();
        state.overlay = Overlay::Bookmarks(crate::reader_state::BookmarksNavigator::default());

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        // Empty state
        terminal.draw(|frame| draw(frame, &state)).unwrap();
        let empty_screen = format!("{}", terminal.backend());
        assert!(empty_screen.contains("Bookmarks"));
        assert!(empty_screen.contains("No bookmarks found"));

        // With bookmarks
        state.bookmarks = vec![crate::storage::Bookmark {
            id: "bm1".into(),
            label: "Chapter 1 Highlight".into(),
            position: TextPosition::new(0, 0),
        }];
        terminal.draw(|frame| draw(frame, &state)).unwrap();
        let populated_screen = format!("{}", terminal.backend());
        assert!(populated_screen.contains("Chapter 1 Highlight"));
        assert!(populated_screen.contains("1 saved bookmark"));
        assert!(populated_screen.contains("Enter jumps · j/k moves · d deletes · Esc closes"));
    }

    #[test]
    fn overlap_context_line_is_rendered_with_muted_style() {
        let mut state = state();
        state.pages[0].overlap_lines = 1;
        state.pages[0].lines = vec!["Context Line".into(), "New Content".into()];
        state.pages[0].anchored_lines = vec![
            crate::pagination::AnchoredLine::new("Context Line".into(), 0, 12, (0..12).collect()),
            crate::pagination::AnchoredLine::new("New Content".into(), 12, 23, (12..23).collect()),
        ];

        let palette = Palette::terminal();
        let rendered = render_reading_lines(&state, palette);
        assert_eq!(rendered.len(), 2);
        // 第 0 行是重叠行，样式应为 muted
        assert_eq!(rendered[0].spans[0].style, palette.muted());
        // 第 1 行是新内容，样式应为 body
        assert_eq!(rendered[1].spans[0].style, palette.body());
    }

    #[test]
    fn footer_renders_auto_reading_status_and_countdown() {
        let mut s = state();
        let palette = Palette::terminal();

        // Initially inactive: no [Auto]
        let normal_footer = reading_footer_line(&s, palette);
        let normal_text: String = normal_footer
            .spans
            .into_iter()
            .map(|sp| sp.content.into_owned())
            .collect();
        assert!(!normal_text.contains("[Auto"));

        // Activate auto reading
        s.update(crate::reader_state::ReaderAction::ToggleAutoReading);
        let active_footer = reading_footer_line(&s, palette);
        let active_text: String = active_footer
            .spans
            .into_iter()
            .map(|sp| sp.content.into_owned())
            .collect();
        assert!(active_text.contains("[Auto: 300 CPM"));

        // Pause auto reading
        s.update(crate::reader_state::ReaderAction::ToggleAutoReadingPause);
        let paused_footer = reading_footer_line(&s, palette);
        let paused_text: String = paused_footer
            .spans
            .into_iter()
            .map(|sp| sp.content.into_owned())
            .collect();
        assert!(paused_text.contains("[Auto: Paused · 300 CPM]"));

        // Narrow terminal (width < 54): still renders exact CPM
        s.viewport.width = 48;
        let narrow_paused = reading_footer_line(&s, palette);
        let narrow_paused_text: String = narrow_paused
            .spans
            .into_iter()
            .map(|sp| sp.content.into_owned())
            .collect();
        assert!(narrow_paused_text.contains("300 CPM"));

        // Agent mode: verify speed is rendered in agent terminal footer
        s.viewport.width = 80;
        s.config.discreet_mode = true;
        s.update(crate::reader_state::ReaderAction::ToggleAutoReadingPause); // resume
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|frame| draw(frame, &s)).unwrap();
        let term_screen = format!("{}", term.backend());
        assert!(term_screen.contains("300 cpm"));
    }
}
