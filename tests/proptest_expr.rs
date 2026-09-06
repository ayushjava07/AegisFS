//! Property-based tests for expression evaluation and template interpolation.

use proptest::prelude::*;
use runvane::engine::expr::{
    interpolate_json, interpolate_string, Expr, ExprContext, ExprError, ExprValue,
};
use serde_json::json;

proptest! {
    #[test]
    fn string_without_placeholders_is_identity(s in "[a-zA-Z0-9 _\\-.,/:]*") {
        let ctx = ExprContext::default();
        let result = interpolate_string(&s, &ctx).expect("plain string interpolation must succeed");
        prop_assert_eq!(result, s);
    }

    #[test]
    fn unclosed_placeholder_syntax_fails(
        prefix in "[a-zA-Z0-9]*",
        var_name in "[a-zA-Z][a-zA-Z0-9_]*",
    ) {
        let malformed = format!("{prefix}${{{var_name}");
        let ctx = ExprContext::default();
        let err = interpolate_string(&malformed, &ctx).unwrap_err();
        let is_syntax = matches!(err, ExprError::Syntax(_));
        prop_assert!(is_syntax);
    }

    #[test]
    fn single_variable_replacement(
        prefix in "[a-z]{0,5}",
        key in "[a-z]{3,10}",
        val in "[0-9]{3,8}",
        suffix in "[a-z]{0,5}",
    ) {
        let ctx = ExprContext::new().with_meta(&key, &val);

        let template = format!("{prefix}${{meta.{key}}}{suffix}");
        let result = interpolate_string(&template, &ctx).expect("valid variable replacement");
        let expected = format!("{prefix}{val}{suffix}");
        prop_assert_eq!(result, expected);
    }

    #[test]
    fn numeric_comparisons_satisfy_algebra(
        a in -10_000.0f64..10_000.0f64,
        b in -10_000.0f64..10_000.0f64,
    ) {
        let ctx = ExprContext::default();
        let expr_lt = Expr::Lt(Box::new(Expr::Literal(ExprValue::Number(a))), Box::new(Expr::Literal(ExprValue::Number(b))));
        let expr_gt = Expr::Gt(Box::new(Expr::Literal(ExprValue::Number(a))), Box::new(Expr::Literal(ExprValue::Number(b))));
        let expr_eq = Expr::Eq(Box::new(Expr::Literal(ExprValue::Number(a))), Box::new(Expr::Literal(ExprValue::Number(b))));

        let res_lt = expr_lt.eval(&ctx).unwrap();
        let res_gt = expr_gt.eval(&ctx).unwrap();
        let res_eq = expr_eq.eval(&ctx).unwrap();

        prop_assert_eq!(res_lt, ExprValue::Bool(a < b));
        prop_assert_eq!(res_gt, ExprValue::Bool(a > b));
        prop_assert_eq!(res_eq, ExprValue::Bool(a == b));
    }

    #[test]
    fn boolean_algebra_invariants(a in any::<bool>(), b in any::<bool>()) {
        let ctx = ExprContext::default();
        let expr_and = Expr::And(Box::new(Expr::Literal(ExprValue::Bool(a))), Box::new(Expr::Literal(ExprValue::Bool(b))));
        let expr_or = Expr::Or(Box::new(Expr::Literal(ExprValue::Bool(a))), Box::new(Expr::Literal(ExprValue::Bool(b))));
        let expr_not = Expr::Not(Box::new(Expr::Literal(ExprValue::Bool(a))));

        prop_assert_eq!(expr_and.eval(&ctx).unwrap(), ExprValue::Bool(a && b));
        prop_assert_eq!(expr_or.eval(&ctx).unwrap(), ExprValue::Bool(a || b));
        prop_assert_eq!(expr_not.eval(&ctx).unwrap(), ExprValue::Bool(!a));
    }

    #[test]
    fn json_primitives_without_templates_are_identity(
        num in any::<i64>(),
        boolean in any::<bool>(),
        text in "[a-zA-Z0-9_]*",
    ) {
        let ctx = ExprContext::default();
        let payload = json!({
            "num": num,
            "bool": boolean,
            "text": text,
            "nested": [num, boolean]
        });

        let interpolated = interpolate_json(&payload, &ctx).expect("valid json interpolation");
        prop_assert_eq!(interpolated, payload);
    }
}
