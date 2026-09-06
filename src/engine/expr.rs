//! Dynamic expression evaluation and template interpolation engine for tasks.
//!
//! Runvane tasks can dynamically consume outputs from upstream tasks or workflow run inputs
//! using template expressions such as `${tasks.upstream_task.output.user_id}` or
//! conditional expressions such as `${tasks.check.output.success} == true`.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// Errors arising during expression parsing or evaluation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExprError {
    /// The expression syntax was malformed.
    #[error("expression syntax error: {0}")]
    Syntax(String),

    /// A referenced variable path was not found in the evaluation context.
    #[error("undefined variable '{0}'")]
    UndefinedVariable(String),

    /// An operation encountered incompatible types.
    #[error("type mismatch: {0}")]
    TypeMismatch(String),

    /// Maximum expression nesting depth was exceeded.
    #[error("maximum expression nesting depth exceeded")]
    NestingLimitExceeded,

    /// Built-in function evaluation failed.
    #[error("function '{name}' error: {message}")]
    FunctionError {
        /// Name of the function.
        name: String,
        /// Description of the evaluation failure.
        message: String,
    },
}

/// Dynamic value type produced during expression evaluation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ExprValue {
    /// Null / empty value.
    Null,
    /// Boolean true or false.
    Bool(bool),
    /// 64-bit floating point number or integer.
    Number(f64),
    /// UTF-8 string value.
    String(String),
    /// Ordered list of dynamic values.
    Array(Vec<ExprValue>),
    /// Key-value mapping of dynamic values.
    Object(BTreeMap<String, ExprValue>),
}

impl From<Json> for ExprValue {
    fn from(json: Json) -> Self {
        match json {
            Json::Null => Self::Null,
            Json::Bool(b) => Self::Bool(b),
            Json::Number(n) => Self::Number(n.as_f64().unwrap_or(0.0)),
            Json::String(s) => Self::String(s),
            Json::Array(arr) => Self::Array(arr.into_iter().map(ExprValue::from).collect()),
            Json::Object(obj) => {
                let mut map = BTreeMap::new();
                for (k, v) in obj {
                    map.insert(k, ExprValue::from(v));
                }
                Self::Object(map)
            }
        }
    }
}

impl From<ExprValue> for Json {
    fn from(val: ExprValue) -> Self {
        match val {
            ExprValue::Null => Json::Null,
            ExprValue::Bool(b) => Json::Bool(b),
            ExprValue::Number(n) => serde_json::Number::from_f64(n)
                .map(Json::Number)
                .unwrap_or(Json::Null),
            ExprValue::String(s) => Json::String(s),
            ExprValue::Array(arr) => Json::Array(arr.into_iter().map(Json::from).collect()),
            ExprValue::Object(obj) => {
                let mut map = serde_json::Map::new();
                for (k, v) in obj {
                    map.insert(k, Json::from(v));
                }
                Json::Object(map)
            }
        }
    }
}

impl fmt::Display for ExprValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => write!(f, "null"),
            Self::Bool(b) => write!(f, "{b}"),
            Self::Number(n) => write!(f, "{n}"),
            Self::String(s) => write!(f, "{s}"),
            Self::Array(arr) => {
                write!(f, "[")?;
                for (i, v) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{v}")?;
                }
                write!(f, "]")
            }
            Self::Object(obj) => {
                write!(f, "{{")?;
                for (i, (k, v)) in obj.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "\"{k}\": {v}")?;
                }
                write!(f, "}}")
            }
        }
    }
}

/// Evaluation context containing workflow run input and upstream task execution outputs.
#[derive(Debug, Clone, Default)]
pub struct ExprContext {
    /// Overall run input payload.
    pub run_input: Json,
    /// Upstream task outputs indexed by task name.
    pub task_outputs: BTreeMap<String, Json>,
    /// System and execution metadata (e.g. `run_id`, `tenant`, `workflow_name`).
    pub meta: BTreeMap<String, String>,
}

impl ExprContext {
    /// Creates an empty evaluation context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the run input payload.
    pub fn with_run_input(mut self, input: Json) -> Self {
        self.run_input = input;
        self
    }

    /// Registers the output of an upstream task.
    pub fn with_task_output(mut self, task_name: impl Into<String>, output: Json) -> Self {
        self.task_outputs.insert(task_name.into(), output);
        self
    }

    /// Sets a context metadata variable.
    pub fn with_meta(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.meta.insert(key.into(), value.into());
        self
    }

    /// Resolves a dotted identifier path (e.g. `tasks.step1.output.id` or `run.input.user.name`).
    pub fn resolve_path(&self, path: &[&str]) -> Result<ExprValue, ExprError> {
        if path.is_empty() {
            return Err(ExprError::Syntax("empty variable path".into()));
        }

        match path[0] {
            "run" => {
                if path.len() >= 2 && path[1] == "input" {
                    let mut curr = &self.run_input;
                    for &segment in &path[2..] {
                        match curr {
                            Json::Object(map) => {
                                curr = map.get(segment).unwrap_or(&Json::Null);
                            }
                            _ => return Ok(ExprValue::Null),
                        }
                    }
                    Ok(ExprValue::from(curr.clone()))
                } else {
                    let full_path = path.join(".");
                    Err(ExprError::UndefinedVariable(full_path))
                }
            }
            "tasks" => {
                if path.len() < 2 {
                    return Err(ExprError::Syntax("incomplete tasks path".into()));
                }
                let task_name = path[1];
                let task_out = match self.task_outputs.get(task_name) {
                    Some(out) => out,
                    None => {
                        let full = path.join(".");
                        return Err(ExprError::UndefinedVariable(full));
                    }
                };

                // Format: tasks.<name>.output.<field>
                let mut curr = task_out;
                let segments = if path.len() >= 3 && path[2] == "output" {
                    &path[3..]
                } else {
                    &path[2..]
                };

                for &segment in segments {
                    match curr {
                        Json::Object(map) => {
                            curr = map.get(segment).unwrap_or(&Json::Null);
                        }
                        _ => return Ok(ExprValue::Null),
                    }
                }
                Ok(ExprValue::from(curr.clone()))
            }
            "meta" => {
                if path.len() == 2 {
                    if let Some(val) = self.meta.get(path[1]) {
                        Ok(ExprValue::String(val.clone()))
                    } else {
                        Ok(ExprValue::Null)
                    }
                } else {
                    let full = path.join(".");
                    Err(ExprError::UndefinedVariable(full))
                }
            }
            other => {
                let full = path.join(".");
                Err(ExprError::UndefinedVariable(format!(
                    "unknown root domain '{other}' in '{full}'"
                )))
            }
        }
    }
}

/// Abstract syntax tree representation of an evaluatable expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Literal constant value.
    Literal(ExprValue),
    /// Variable path reference (e.g. `tasks.step1.output.id`).
    Variable(Vec<String>),
    /// Equality comparison `a == b`.
    Eq(Box<Expr>, Box<Expr>),
    /// Inequality comparison `a != b`.
    Ne(Box<Expr>, Box<Expr>),
    /// Less than comparison `a < b`.
    Lt(Box<Expr>, Box<Expr>),
    /// Greater than comparison `a > b`.
    Gt(Box<Expr>, Box<Expr>),
    /// Logical AND `a && b`.
    And(Box<Expr>, Box<Expr>),
    /// Logical OR `a || b`.
    Or(Box<Expr>, Box<Expr>),
    /// Logical NOT `!a`.
    Not(Box<Expr>),
}

impl Expr {
    /// Evaluates the expression against the provided context.
    pub fn eval(&self, ctx: &ExprContext) -> Result<ExprValue, ExprError> {
        self.eval_depth(ctx, 0)
    }

    fn eval_depth(&self, ctx: &ExprContext, depth: usize) -> Result<ExprValue, ExprError> {
        if depth > 32 {
            return Err(ExprError::NestingLimitExceeded);
        }

        match self {
            Self::Literal(val) => Ok(val.clone()),
            Self::Variable(path) => {
                let segments: Vec<&str> = path.iter().map(String::as_str).collect();
                ctx.resolve_path(&segments)
            }
            Self::Eq(left, right) => {
                let l = left.eval_depth(ctx, depth + 1)?;
                let r = right.eval_depth(ctx, depth + 1)?;
                Ok(ExprValue::Bool(l == r))
            }
            Self::Ne(left, right) => {
                let l = left.eval_depth(ctx, depth + 1)?;
                let r = right.eval_depth(ctx, depth + 1)?;
                Ok(ExprValue::Bool(l != r))
            }
            Self::Lt(left, right) => {
                let l = left.eval_depth(ctx, depth + 1)?;
                let r = right.eval_depth(ctx, depth + 1)?;
                match (l, r) {
                    (ExprValue::Number(a), ExprValue::Number(b)) => Ok(ExprValue::Bool(a < b)),
                    (ExprValue::String(a), ExprValue::String(b)) => Ok(ExprValue::Bool(a < b)),
                    _ => Err(ExprError::TypeMismatch(
                        "< requires comparable operands".into(),
                    )),
                }
            }
            Self::Gt(left, right) => {
                let l = left.eval_depth(ctx, depth + 1)?;
                let r = right.eval_depth(ctx, depth + 1)?;
                match (l, r) {
                    (ExprValue::Number(a), ExprValue::Number(b)) => Ok(ExprValue::Bool(a > b)),
                    (ExprValue::String(a), ExprValue::String(b)) => Ok(ExprValue::Bool(a > b)),
                    _ => Err(ExprError::TypeMismatch(
                        "> requires comparable operands".into(),
                    )),
                }
            }
            Self::And(left, right) => {
                let l = left.eval_depth(ctx, depth + 1)?;
                match l {
                    ExprValue::Bool(false) => Ok(ExprValue::Bool(false)),
                    ExprValue::Bool(true) => {
                        let r = right.eval_depth(ctx, depth + 1)?;
                        match r {
                            ExprValue::Bool(b) => Ok(ExprValue::Bool(b)),
                            _ => Err(ExprError::TypeMismatch(
                                "&& requires boolean operands".into(),
                            )),
                        }
                    }
                    _ => Err(ExprError::TypeMismatch(
                        "&& requires boolean operands".into(),
                    )),
                }
            }
            Self::Or(left, right) => {
                let l = left.eval_depth(ctx, depth + 1)?;
                match l {
                    ExprValue::Bool(true) => Ok(ExprValue::Bool(true)),
                    ExprValue::Bool(false) => {
                        let r = right.eval_depth(ctx, depth + 1)?;
                        match r {
                            ExprValue::Bool(b) => Ok(ExprValue::Bool(b)),
                            _ => Err(ExprError::TypeMismatch(
                                "|| requires boolean operands".into(),
                            )),
                        }
                    }
                    _ => Err(ExprError::TypeMismatch(
                        "|| requires boolean operands".into(),
                    )),
                }
            }
            Self::Not(inner) => {
                let val = inner.eval_depth(ctx, depth + 1)?;
                match val {
                    ExprValue::Bool(b) => Ok(ExprValue::Bool(!b)),
                    _ => Err(ExprError::TypeMismatch("! requires boolean operand".into())),
                }
            }
        }
    }
}

/// Interpolates `${...}` expressions embedded in a template string.
pub fn interpolate_string(template: &str, ctx: &ExprContext) -> Result<String, ExprError> {
    let mut out = String::with_capacity(template.len());
    let mut chars = template.char_indices().peekable();

    while let Some((i, ch)) = chars.next() {
        if ch == '$' && chars.peek().map(|&(_, next)| next) == Some('{') {
            chars.next(); // consume '{'
            let start = i + 2;
            let mut end = None;

            for (close_idx, close_ch) in chars.by_ref() {
                if close_ch == '}' {
                    end = Some(close_idx);
                    break;
                }
            }

            let close_pos = end.ok_or_else(|| {
                ExprError::Syntax(format!(
                    "unclosed '${{' expression starting at index {start}"
                ))
            })?;

            let var_name = template[start..close_pos].trim();
            let segments: Vec<&str> = var_name.split('.').collect();
            let val = ctx.resolve_path(&segments)?;
            out.push_str(&val.to_string());
        } else {
            out.push(ch);
        }
    }

    Ok(out)
}

/// Recursively interpolates any strings inside a JSON value using the given context.
pub fn interpolate_json(value: &Json, ctx: &ExprContext) -> Result<Json, ExprError> {
    match value {
        Json::String(s) => {
            // Check if string is an exact `${...}` variable expression
            let trimmed = s.trim();
            if trimmed.starts_with("${")
                && trimmed.ends_with('}')
                && trimmed[2..trimmed.len() - 1].find("${").is_none()
            {
                let inner = &trimmed[2..trimmed.len() - 1].trim();
                let segments: Vec<&str> = inner.split('.').collect();
                let val = ctx.resolve_path(&segments)?;
                return Ok(Json::from(val));
            }
            let interpolated = interpolate_string(s, ctx)?;
            Ok(Json::String(interpolated))
        }
        Json::Array(arr) => {
            let mut out = Vec::with_capacity(arr.len());
            for item in arr {
                out.push(interpolate_json(item, ctx)?);
            }
            Ok(Json::Array(out))
        }
        Json::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                out.insert(k.clone(), interpolate_json(v, ctx)?);
            }
            Ok(Json::Object(out))
        }
        other => Ok(other.clone()),
    }
}
