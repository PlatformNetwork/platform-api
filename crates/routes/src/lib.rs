//! HTTP route handlers for the platform API
//!
//! This crate contains all HTTP route handlers organized by functional area.

pub mod attestation;
pub mod challenge_credentials;
pub mod challenge_proxy;
pub mod challenges;
pub mod config;
pub mod emissions;
pub mod health;
pub mod jobs;
pub mod mechanisms;
pub mod metagraph;
pub mod network;
pub mod orm;
pub mod results;
pub mod ui;
pub mod validators;
pub mod websocket;

