//! Safe macOS audit and optimization for slow Intel Macs.
//!
//! Everything here is user-level (no root): visual effects (`defaults`) and the
//! user's third-party LaunchAgents (`launchctl`). Every change goes into a
//! journal at `~/Library/Application Support/dev.intelmactuner/` and is reversible.
//!
//! Collection and the actions (running commands) are isolated; classification,
//! the journal and formatting are pure and tested. Used by the CLI and the menu bar.

mod catalog;
mod collect;
mod daemon;
mod exec;
mod journal;
mod model;
mod parse;
mod report;
mod trim;

pub use self::catalog::*;
pub use self::collect::*;
pub use self::daemon::*;
use self::exec::*;
pub use self::journal::*;
pub use self::model::*;
pub use self::parse::*;
pub use self::trim::*;
