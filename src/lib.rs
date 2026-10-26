//! Bidirectional converter between HWPX documents and Markdown.
//!
//! Supports HWPX (ZIP/XML-based) as input or output, with a
//! language-agnostic intermediate representation ([`ir`]) in between.  The
//! legacy binary HWP 5.0 format (`.hwp`) is not supported; save such
//! documents as `.hwpx` in Hancom Office first.
//!
//! # Quick start
//!
//! ## Using the builder API (recommended for library users)
//!
//! ```no_run
//! use std::path::Path;
//! use hwp2md::convert::ConvertOptions;
//!
//! // HWPX → Markdown (direction inferred from extensions)
//! ConvertOptions::new(Path::new("document.hwpx"), Path::new("document.md"))
//!     .frontmatter(true)
//!     .execute()
//!     .expect("conversion failed");
//!
//! // Markdown → HWPX, overwrite if output exists
//! ConvertOptions::new(Path::new("document.md"), Path::new("document.hwpx"))
//!     .force(true)
//!     .execute()
//!     .expect("conversion failed");
//! ```
//!
//! ## Using the low-level functions
//!
//! ```no_run
//! use std::path::Path;
//!
//! // HWPX → Markdown
//! hwp2md::convert::to_markdown(
//!     Path::new("document.hwpx"),
//!     Some(Path::new("document.md")),
//!     None,   // assets_dir
//!     false,  // frontmatter
//! ).expect("conversion failed");
//!
//! // Markdown → HWPX (deprecated: no longer developed)
//! # #[allow(deprecated)]
//! hwp2md::convert::to_hwpx(
//!     Path::new("document.md"),
//!     Some(Path::new("document.hwpx")),
//!     None,   // style template
//! ).expect("conversion failed");
//! ```
//!
//! # Modules
//!
//! | Module | Purpose |
//! |--------|---------|
//! | [`convert`] | Top-level conversion entry points |
//! | [`ir`] | Intermediate representation (Document, Block, Inline, …) |
//! | [`error`] | [`Hwp2MdError`] error type |
//! | [`hwpx`] | HWPX reader and writer (ZIP/XML) |
//! | [`md`] | Markdown reader and writer |
//!
//! See the [README](https://github.com/hephaex/hwp2md#readme) for detailed
//! usage, CLI reference, and format support matrix.
#![warn(missing_docs)]

pub mod convert;
pub mod error;
pub(crate) mod heading_style;
pub mod hwpx;
pub mod ir;
pub mod md;
pub mod style;
pub(crate) mod url_util;

pub use convert::ConvertOptions;
pub use error::Hwp2MdError;
