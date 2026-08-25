#![forbid(unsafe_code)]

//! Optional optimization crate.
//!
//! # Role in FrankenTUI
//! `ftui-simd` is a sandbox for safe, performance-oriented code paths. It
//! allows experimentation with autovec-friendly loops and portable SIMD without
//! imposing dependencies on the core crates.
//!
//! # How it fits in the system
//! This crate is optional and should be wired in via feature flags. The render
//! kernel or other performance-sensitive components can call into this crate
//! when enabled, but the base system remains fully functional without it.
//!
//! Note: This project currently forbids unsafe code. This crate exists to host
//! safe optimizations behind feature flags without impacting the rest of the
//! workspace.
