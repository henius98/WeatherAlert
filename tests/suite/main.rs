//! Central test suite.
//!
//! Every test in the project lives under `tests/suite/`, one file per domain
//! module, compiled into a single binary. `src/` holds no test code.
//!
//! These are integration tests: they see only the public API, which is why the
//! domain layer carries no `worker` dependency and can be exercised natively.

mod commands;
mod daily;
mod errors;
mod forecast;
mod met;
mod quota;
mod render;
mod rules;
mod state;
mod tiers;
mod timeutil;
mod tomorrow;
mod util;
