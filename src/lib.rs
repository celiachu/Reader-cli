mod atomic_file;
pub mod auto_reading;
pub mod book;
pub mod bookshelf;
pub mod cli;
pub mod config;
pub mod discreet;
pub mod hotkey;
pub mod network;
pub mod network_shelf;
pub mod pagination;
pub mod paths;
#[deprecated(note = "use reader::discreet")]
pub mod privacy;
mod reader_input;
mod reader_state;
mod reader_view;
mod reader_writer;
pub mod reading_data;
pub mod shelf;
pub mod storage;
pub mod tui;
pub mod ui;

pub use book::{
    BookDocument, BookFormat, ContentMetrics, ContentsEntry, ContentsSource, Section, TextPosition,
};
pub use config::ReaderConfig;
pub use discreet::discreet_stack;
pub use pagination::{paginate, Page};
pub use paths::{configuration_path, library_path, reading_data_dir};
#[allow(deprecated)]
pub use privacy::privacy_stack;
pub use reading_data::{
    BookmarkOperation, BookmarkOperationAction, ReadingDataStore, ReadingEvent, SessionCheckpoint,
    SessionTracker, StoredReadingEvent,
};
pub use storage::{Bookmark, Library, LibraryBook, Progress, ScanDirResult, Store};
