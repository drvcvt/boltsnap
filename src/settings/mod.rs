//! The settings in `config.toml`: which keys exist, reading them with a report
//! of what is wrong, changing single keys without touching the rest, and the
//! JSON the daemon speaks about them.

pub mod load;
pub mod schema;
pub mod state;
pub mod write;

pub use load::{Doc, Issue, Loaded, Value, load};
pub use state::{Broken, State};
pub use write::{Change, apply, update, write};

#[cfg(test)]
mod tests;
