//! Secure ORM Gateway for challenge database access
//!
//! This crate provides a secure ORM gateway that allows challenges to execute
//! SQL queries with schema isolation and permission validation.

mod executor;
mod mod_rs;
pub mod permissions;
pub mod query_validator;

pub use executor::QueryExecutor;
pub use mod_rs::*;
pub use permissions::{ORMPermissions, TablePermission};
pub use query_validator::QueryValidator;

