use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyEvent, KeyEventKind};
use ratatui::layout::Rect;

use crate::book::BookDocument;
use crate::config::ReaderConfig;
use crate::reader_input::action_for_key;
use crate::reader_state::{NotificationKind, Overlay, ReaderAction, ReaderEffect, ReaderState};
use crate::reader_writer::{ReaderWriter, WriteCommand};
use crate::reading_data::{ReadingDataStore, SessionTracker};
use crate::storage::Store;

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

pub fn run_reader(
    document: BookDocument,
    book_id: String,
    store: &mut Store,
    config: &mut ReaderConfig,
) -> Result<()> {
    let data = ReadingDataStore::open_default(config)?;
    run_reader_with_data(document, book_id, store, config, data)
}

fn run_reader_with_data(
    document: BookDocument,
    book_id: String,
    store: &mut Store,
    config: &mut ReaderConfig,
    data: ReadingDataStore,
) -> Result<()> {
    let sync_report = data.synchronize_store(store)?;
    for warning in sync_report.warnings {
        eprintln!("warning: {warning}");
    }
    if sync_report.state_changed {
        store.save()?;
    }

    let mut terminal = ratatui::init();
    let _guard = TerminalGuard;
    let size = terminal.size()?;
    let viewport = Rect::new(0, 0, size.width, size.height);

    store.update_document_metrics(&book_id, &document);
    let saved = store.progress_for(&book_id).copied().unwrap_or_default();
    let bookmarks = store.bookmarks_for(&book_id).to_vec();
    let restored = store
        .book_by_id(&book_id)
        .is_some_and(|book| book.last_opened.is_some())
        || saved.position != crate::book::TextPosition::default();
    store.record_open(&book_id);
    let mut state = ReaderState::new(
        document,
        book_id,
        viewport,
        config.clone(),
        saved,
        bookmarks,
    );
    // A library entry always has a default Progress, so presence of that
    // field alone does not prove that this is a resumed reading session.
    state.agent.resumed = restored;
    let writer = ReaderWriter::start(store.clone(), config.clone(), data, state.book_id.clone())
        .map_err(|_| anyhow::anyhow!("Unable to start local persistence"))?;
    let (mut session, initial_checkpoint) = SessionTracker::start(
        state.book_id.clone(),
        state.position,
        time::OffsetDateTime::now_utc(),
        Instant::now(),
    );
    enqueue(
        &writer,
        WriteCommand::Session(initial_checkpoint),
        &mut state,
    );
    enqueue(
        &writer,
        WriteCommand::Progress(state.current_progress()),
        &mut state,
    );
    let mut exit = false;
    let (_hotkey_listener, hotkey_rx) = crate::hotkey::GlobalHotkeyListener::start_f12();
    let mut last_f12_toggle = Instant::now() - Duration::from_secs(1);

    while !exit {
        apply_receipts(&writer, &mut state);
        let now = Instant::now();
        if !state.config.discreet_mode {
            state.expire_notification(now);
        }
        if let Some(checkpoint) = session.tick(state.position, time::OffsetDateTime::now_utc(), now)
        {
            enqueue(&writer, WriteCommand::Session(checkpoint), &mut state);
        }
        terminal.draw(|frame| crate::reader_view::draw(frame, &state))?;

        let mut global_f12 = false;
        while let Ok(()) = hotkey_rx.try_recv() {
            global_f12 = true;
        }

        let mut action = if global_f12 {
            if now.duration_since(last_f12_toggle) >= Duration::from_millis(250) {
                last_f12_toggle = now;
                Some(ReaderAction::ToggleHidden)
            } else {
                None
            }
        } else {
            None
        };

        if action.is_none() && event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) if should_handle_key_event(key) => {
                    let key_action = action_for_key(key, &state);
                    if matches!(key_action, ReaderAction::ToggleHidden) {
                        if now.duration_since(last_f12_toggle) >= Duration::from_millis(250) {
                            last_f12_toggle = now;
                            action = Some(ReaderAction::ToggleHidden);
                        }
                    } else if !matches!(key_action, ReaderAction::None) {
                        action = Some(key_action);
                    }
                }
                Event::Resize(width, height) => {
                    state.resize(width, height);
                    if state.hidden {
                        terminal.clear()?;
                    }
                }
                _ => {}
            }
        }

        if action.is_none() {
            action = state.auto_reading_tick(now);
        }

        let Some(action) = action else {
            continue;
        };

        let was_hidden = state.hidden;
        let previous_position = state.position;
        let effects = state.update(action);
        // Also catches :fold, not just the physical F12 key. No disk
        // work, session finalization or notification precedes this frame.
        // Differential redraw in place without terminal.clear() to avoid full-screen flash.
        if state.hidden != was_hidden {
            terminal.draw(|frame| crate::reader_view::draw(frame, &state))?;
        }
        let wall = time::OffsetDateTime::now_utc();
        let now = Instant::now();
        let checkpoint = if state.hidden && !was_hidden {
            session.finish(state.position, wall, now)
        } else if !state.hidden && (was_hidden || !matches!(action, ReaderAction::None)) {
            session.on_activity(previous_position, wall, now)
        } else {
            None
        };
        if let Some(checkpoint) = checkpoint {
            enqueue(&writer, WriteCommand::Session(checkpoint), &mut state);
        }
        for effect in effects {
            match effect {
                ReaderEffect::SaveProgress(progress) => {
                    enqueue(&writer, WriteCommand::Progress(progress), &mut state);
                }
                ReaderEffect::SaveConfig => {
                    enqueue(
                        &writer,
                        WriteCommand::Config(state.config.clone()),
                        &mut state,
                    );
                }
                ReaderEffect::AddBookmark { label, progress } => {
                    state.agent.receipt = Some("Checkpoint saving".into());
                    enqueue(&writer, WriteCommand::Add { label, progress }, &mut state);
                }
                ReaderEffect::DeleteBookmark { id, removed, index } => {
                    if !writer.enqueue(WriteCommand::Delete(id)) {
                        let insert_at = index.min(state.bookmarks.len());
                        state.bookmarks.insert(insert_at, removed);
                        if let Overlay::Bookmarks(navigator) = &mut state.overlay {
                            navigator.selection = insert_at;
                        }
                        state.agent.receipt = Some("Save queue busy; retry the command".into());
                        if !state.hidden && !state.config.discreet_mode {
                            state.notify(
                                "Save queue busy; retry the command",
                                NotificationKind::Error,
                            );
                        }
                    }
                }
                ReaderEffect::Exit => {
                    exit = true;
                }
            }
        }
        if !state.hidden && state.position != previous_position {
            if let Some(checkpoint) = session.checkpoint(state.position, wall, now, true) {
                enqueue(&writer, WriteCommand::Session(checkpoint), &mut state);
            }
        }
    }
    if let Some(checkpoint) = session.finish(
        state.position,
        time::OffsetDateTime::now_utc(),
        Instant::now(),
    ) {
        enqueue(&writer, WriteCommand::Session(checkpoint), &mut state);
    }
    *config = state.config.clone();
    let outcome = writer.finish(state.current_progress(), state.config.clone())?;
    *store = outcome.store;
    if !outcome.flushed {
        anyhow::bail!("Local state could not be saved");
    }
    if outcome.had_errors {
        anyhow::bail!("A local save operation failed during this session");
    }
    Ok(())
}

fn enqueue(writer: &ReaderWriter, command: WriteCommand, state: &mut ReaderState) {
    if !writer.enqueue(command) {
        state.agent.receipt = Some("Save queue busy; retry the command".into());
        if !state.hidden && !state.config.discreet_mode {
            state.notify(
                "Save queue busy; retry the command",
                NotificationKind::Error,
            );
        }
    }
}

fn apply_receipts(writer: &ReaderWriter, state: &mut ReaderState) {
    for receipt in writer.receipts.try_iter() {
        if let Some(bookmarks) = receipt.bookmarks {
            state.bookmarks = bookmarks;
        }
        if receipt.checkpoint_saved {
            state.agent.checkpoints = state.agent.checkpoints.saturating_add(1);
        }
        if receipt.text.is_empty() {
            continue;
        }
        // Do not resurrect notifications after folding. Errors remain available
        // as a neutral status on restore, but never enter the safe renderer.
        if !state.hidden || receipt.error {
            state.agent.receipt = Some(receipt.text.into());
        }
        if !state.hidden && !state.config.discreet_mode {
            state.notify(
                receipt.text,
                if receipt.error {
                    NotificationKind::Error
                } else {
                    NotificationKind::Success
                },
            );
        }
    }
}

fn should_handle_key_event(key: KeyEvent) -> bool {
    key.kind == KeyEventKind::Press
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    /// Real-terminal fixture: runs the production loop with entirely isolated
    /// stores. The external ConPTY driver supplies keystrokes and screenshots.
    #[test]
    #[ignore = "interactive ConPTY fixture; requires READER_TERMINAL_FIXTURE"]
    fn terminal_fixture() {
        let root = std::path::PathBuf::from(
            std::env::var_os("READER_TERMINAL_FIXTURE").expect("fixture directory"),
        );
        let document =
            crate::book::load_book(root.join("SECRET_TITLE - SECRET_AUTHOR.txt")).unwrap();
        let mut store = Store::open(root.join("library.json")).unwrap();
        let id = store
            .add_book(&document, &document.path)
            .unwrap()
            .id
            .clone();
        store.record_open(&id);
        let mut config = ReaderConfig::open(root.join("configuration.json")).unwrap();
        config.discreet_mode = true;
        let data = ReadingDataStore::new(root.join("journal"), None);
        run_reader_with_data(document, id, &mut store, &mut config, data).unwrap();
    }

    #[test]
    fn only_key_presses_reach_the_state_machine() {
        assert!(should_handle_key_event(KeyEvent::new(
            KeyCode::Char('g'),
            KeyModifiers::NONE
        )));
        assert!(!should_handle_key_event(KeyEvent::new_with_kind(
            KeyCode::Char('g'),
            KeyModifiers::NONE,
            KeyEventKind::Release
        )));
    }
}
