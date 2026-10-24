//! The settings in `config.toml`: which keys exist, reading them with a report
//! of what is wrong, and changing single keys without touching the rest.

pub mod load;
pub mod schema;
pub mod write;

pub use load::{Doc, Issue, Loaded, Value, load};
pub use write::{Change, apply, write};

#[cfg(test)]
mod tests;
