//! Comprehensive unit and boundary tests for the workflow expression engine.

use serde_json::json;

use super::expr::*;

#[test]
fn context_resolves_run_input_paths() {
    let ctx = ExprContext::new().with_run_input(json!({
        "user": {
            "id": 12345,
            "name": "alice",
            "active": true
        },
        "tags": ["prod", "us-east"]
    }));

    let id = ctx.resolve_path(&["run", "input", "user", "id"]).unwrap();
    assert_eq!(id, ExprValue::Number(12345.0));

    let name = ctx.resolve_path(&["run", "input", "user", "name"]).unwrap();
    assert_eq!(name, ExprValue::String("alice".into()));

    let active = ctx
        .resolve_path(&["run", "input", "user", "active"])
        .unwrap();
    assert_eq!(active, ExprValue::Bool(true));

    let missing = ctx
        .resolve_path(&["run", "input", "user", "missing"])
        .unwrap();
    assert_eq!(missing, ExprValue::Null);
}

#[test]
fn context_resolves_task_outputs() {
    let ctx = ExprContext::new().with_task_output(
        "query_db",
        json!({
            "rows_affected": 42,
            "status": "completed"
        }),
    );

    let rows = ctx
        .resolve_path(&["tasks", "query_db", "output", "rows_affected"])
        .unwrap();
    assert_eq!(rows, ExprValue::Number(42.0));

    // Shorthand without "output" literal
    let status = ctx.resolve_path(&["tasks", "query_db", "status"]).unwrap();
    assert_eq!(status, ExprValue::String("completed".into()));

    // Unknown task returns error
    let err = ctx
        .resolve_path(&["tasks", "unknown_step", "out"])
        .unwrap_err();
    assert!(matches!(err, ExprError::UndefinedVariable(_)));
}

#[test]
fn string_template_interpolation() {
    let ctx = ExprContext::new()
        .with_run_input(json!({ "env": "staging" }))
        .with_task_output("fetch", json!({ "count": 7 }));

    let interpolated = interpolate_string(
        "Deploying to ${run.input.env} with ${tasks.fetch.output.count} instances",
        &ctx,
    )
    .unwrap();

    assert_eq!(interpolated, "Deploying to staging with 7 instances");
}

#[test]
fn json_template_interpolation_exact_and_embedded() {
    let ctx = ExprContext::new()
        .with_run_input(json!({ "enabled": true, "threshold": 88 }))
        .with_task_output("step1", json!({ "data": { "val": "abc" } }));

    let template = json!({
        "flag": "${run.input.enabled}",
        "limit": "${run.input.threshold}",
        "msg": "Hello ${tasks.step1.output.data.val}!"
    });

    let result = interpolate_json(&template, &ctx).unwrap();
    assert_eq!(result["flag"], json!(true));
    assert_eq!(result["limit"], json!(88.0));
    assert_eq!(result["msg"], json!("Hello abc!"));
}

#[test]
fn boolean_and_comparison_expressions() {
    let ctx = ExprContext::new()
        .with_run_input(json!({ "score": 95, "admin": true }))
        .with_task_output("check", json!({ "passed": true }));

    // score > 90
    let expr_gt = Expr::Gt(
        Box::new(Expr::Variable(vec![
            "run".into(),
            "input".into(),
            "score".into(),
        ])),
        Box::new(Expr::Literal(ExprValue::Number(90.0))),
    );
    assert_eq!(expr_gt.eval(&ctx).unwrap(), ExprValue::Bool(true));

    // passed == true && admin == true
    let expr_and = Expr::And(
        Box::new(Expr::Eq(
            Box::new(Expr::Variable(vec![
                "tasks".into(),
                "check".into(),
                "passed".into(),
            ])),
            Box::new(Expr::Literal(ExprValue::Bool(true))),
        )),
        Box::new(Expr::Eq(
            Box::new(Expr::Variable(vec![
                "run".into(),
                "input".into(),
                "admin".into(),
            ])),
            Box::new(Expr::Literal(ExprValue::Bool(true))),
        )),
    );
    assert_eq!(expr_and.eval(&ctx).unwrap(), ExprValue::Bool(true));

    // !admin
    let expr_not = Expr::Not(Box::new(Expr::Variable(vec![
        "run".into(),
        "input".into(),
        "admin".into(),
    ])));
    assert_eq!(expr_not.eval(&ctx).unwrap(), ExprValue::Bool(false));
}

#[test]
fn unclosed_expression_syntax_error() {
    let ctx = ExprContext::new();
    let err = interpolate_string("Hello ${unclosed", &ctx).unwrap_err();
    assert!(matches!(err, ExprError::Syntax(_)));
}
