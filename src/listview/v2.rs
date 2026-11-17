//! Candidate for the virtualized list view that will replace this module's widget.
//!
//! It only builds for debug tests until it is complete, because the headless tests
//! use iced's null renderer, which only exists in debug builds.

mod heights;
mod model;
