#![deny(warnings, clippy::pedantic)]

#[allow(clippy::module_inception)]
mod filter;
mod matcher;
pub mod regex;
mod resolver;

pub use filter::*;
pub use matcher::*;
pub use resolver::*;

use super::search::{BooleanType, Field, QueryNode, normalize_fields};
