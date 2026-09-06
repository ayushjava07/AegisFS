//! Workflow execution engine, expression parsing, and templating.
//!
//! Sub-modules:
//! * [`expr`] — dynamic expression evaluator and string template interpolation.

pub mod expr;

#[cfg(test)]
mod tests;
