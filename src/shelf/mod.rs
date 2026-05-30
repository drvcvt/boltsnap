pub mod layout;
pub mod model;
pub mod paint;
pub mod thumbnail;

use crate::DynResult;

/// Run the shelf daemon (filled in Milestone D).
pub fn run_daemon() -> DynResult<()> {
    eprintln!("boltsnap daemon: not yet implemented");
    Ok(())
}
