//! Terminal map view for Pathify.
//!
//! Kept as a separate crate so that `ratatui` and `crossterm` never enter the
//! dependency graph of the one-shot, pipeline-friendly commands, and so the
//! spatial logic in `pathify-core` stays testable without a terminal.
//!
//! Not implemented yet: `pathify view` lands with the interactive
//! braille-canvas map. The crate exists now to hold that boundary in place.
