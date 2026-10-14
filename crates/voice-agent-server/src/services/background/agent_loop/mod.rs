//! Bounded two-level Agent Job execution. Rust owns every accepted action.
mod child;
mod completion;
mod context;
mod delegation;
mod results;
mod root;
mod tools;

pub(super) use root::run_root;

#[cfg(test)]
mod tests;
