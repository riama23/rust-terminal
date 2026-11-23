//! Integración con Claude Code (v1.2): broker de hooks y status line.
//!
//! Hasta que la interfaz (sidebar, attention bar) lo use, parte de la API
//! pública no tiene llamadores.
#![allow(dead_code)]

pub mod broker;
pub mod http;
pub mod hub;
pub mod launch;
pub mod model;
pub mod rules;

#[cfg(test)]
mod e2e;
