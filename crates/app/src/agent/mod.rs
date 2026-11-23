//! Integración con Claude Code (v1.2): broker de hooks y status line.
//!
//! Hasta que la interfaz (sidebar, attention bar) lo use, parte de la API
//! pública no tiene llamadores.
#![allow(dead_code)]

pub mod alerts;
pub mod broker;
pub mod diff;
pub mod git;
pub mod http;
pub mod hub;
pub mod launch;
pub mod model;
pub mod review;
pub mod rules;

#[cfg(test)]
mod e2e;
