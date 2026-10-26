//! HWPX reader and writer (ZIP/XML format).
#![allow(missing_docs)]

mod reader;
mod table_layout;
mod writer;

pub use reader::read_hwpx;
#[allow(deprecated)] // re-export; the item carries the deprecation
pub use writer::write_hwpx;
