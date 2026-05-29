//! Convert the GX AST (Rust types) into GX runtime Values.
//! Used by the `parse_gx` builtin so GX programs can inspect their own AST.

use crate::ast::*;
use crate::value::Value;
use std::collections::HashMap;

pub(super) fn gx_ast_to_value(program: &Program) -> Value {
    let mut stmts: Vec<Value> = Vec::new();
    for f in &program.functions {
        stmts.push(funcdef_to_value(&f.name, &f.params, &f.body));
    }
    if let Some(brain) = &program.top_level_brain {
        for s in &brain.plan {
            stmts.push(stmt_to_value(s));
        }
        for s in &brain.execute {
            stmts.push(stmt_to_value(s));
        }
        for s in &brain.remember {
            stmts.push(stmt_to_value(s));
        }
        for s in &brain.communicate {
            stmts.push(stmt_to_value(s));
        }
    }
    for h in &program.helpers {
        for f in &h.recipes {
            stmts.push(funcdef_to_value(&f.name, &[], &f.brain.plan));
        }
        for wb in &h.when_blocks {
            if matches!(wb.trigger, WhenTrigger::Started) {
                for s in &wb.body {
                    stmts.push(stmt_to_value(s));
                }
            }
        }
    }
    obj(&[
        ("tag", Value::Str("Program".into())),
        ("stmts", Value::Array(stmts)),
    ])
}

fn funcdef_to_value(name: &str, params: &[String], body: &[Stmt]) -> Value {
    obj(&[
        ("tag", Value::Str("FuncDef".into())),
        ("name", Value::Str(name.into())),
        (
            "params",
            Value::Array(params.iter().map(|p| Value::Str(p.clone())).collect()),
        ),
        ("body", stmts_to_value(body)),
    ])
}

fn stmts_to_value(stmts: &[Stmt]) -> Value {
    Value::Array(stmts.iter().map(stmt_to_value).collect())
}

fn stmt_to_value(s: &Stmt) -> Value {
    match s {
        Stmt::Assign { target, value, .. } => obj(&[
            ("tag", Value::Str("Assign".into())),
            ("target", expr_to_value(target)),
            ("value", expr_to_value(value)),
        ]),
        Stmt::PlusAssign { target, value, .. } => obj(&[
            ("tag", Value::Str("PlusEq".into())),
            ("target", expr_to_value(target)),
            ("value", expr_to_value(value)),
        ]),
        Stmt::MinusAssign { target, value, .. } => obj(&[
            ("tag", Value::Str("MinusEq".into())),
            ("target", expr_to_value(target)),
            ("value", expr_to_value(value)),
        ]),
        Stmt::MulAssign { target, value, .. } => obj(&[
            ("tag", Value::Str("MulEq".into())),
            ("target", expr_to_value(target)),
            ("value", expr_to_value(value)),
        ]),
        Stmt::DivAssign { target, value, .. } => obj(&[
            ("tag", Value::Str("DivEq".into())),
            ("target", expr_to_value(target)),
            ("value", expr_to_value(value)),
        ]),
        Stmt::If {
            branches,
            else_body,
            ..
        } => {
            let br_vals: Vec<Value> = branches
                .iter()
                .map(|(cond, body)| {
                    obj(&[
                        ("cond", expr_to_value(cond)),
                        ("body", stmts_to_value(body)),
                    ])
                })
                .collect();
            obj(&[
                ("tag", Value::Str("If".into())),
                ("branches", Value::Array(br_vals)),
                (
                    "else_body",
                    else_body
                        .as_ref()
                        .map(|b| stmts_to_value(b))
                        .unwrap_or(Value::Null),
                ),
            ])
        }
        Stmt::While {
            condition, body, ..
        } => obj(&[
            ("tag", Value::Str("While".into())),
            ("cond", expr_to_value(condition)),
            ("body", stmts_to_value(body)),
        ]),
        Stmt::ForEach {
            var, iter, body, ..
        } => obj(&[
            ("tag", Value::Str("For".into())),
            ("var", Value::Str(var.clone())),
            ("iter", expr_to_value(iter)),
            ("body", stmts_to_value(body)),
        ]),
        Stmt::Return { value, .. } => obj(&[
            ("tag", Value::Str("Return".into())),
            (
                "value",
                value.as_ref().map(expr_to_value).unwrap_or(Value::Null),
            ),
        ]),
        Stmt::Break { .. } => obj(&[("tag", Value::Str("Break".into()))]),
        Stmt::Continue { .. } => obj(&[("tag", Value::Str("Continue".into()))]),
        Stmt::Log { value, .. } | Stmt::Output { value, .. } => obj(&[
            ("tag", Value::Str("Log".into())),
            ("value", expr_to_value(value)),
        ]),
        Stmt::Say { value, .. } => obj(&[
            ("tag", Value::Str("Say".into())),
            ("value", expr_to_value(value)),
        ]),
        Stmt::Assert {
            condition, message, ..
        } => obj(&[
            ("tag", Value::Str("Assert".into())),
            ("cond", expr_to_value(condition)),
            (
                "msg",
                message.as_ref().map(expr_to_value).unwrap_or(Value::Null),
            ),
        ]),
        Stmt::TryCatch {
            try_body,
            catch_var,
            catch_body,
            ..
        } => obj(&[
            ("tag", Value::Str("TryCatch".into())),
            ("try_body", stmts_to_value(try_body)),
            ("catch_var", Value::Str(catch_var.clone())),
            ("catch_body", stmts_to_value(catch_body)),
        ]),
        Stmt::Expr { expr, .. } => obj(&[
            ("tag", Value::Str("ExprStmt".into())),
            ("expr", expr_to_value(expr)),
        ]),
        _ => obj(&[
            ("tag", Value::Str("ExprStmt".into())),
            ("expr", Value::Null),
        ]),
    }
}

fn expr_to_value(e: &Expr) -> Value {
    match e {
        Expr::Num(n) => obj(&[
            ("tag", Value::Str("Num".into())),
            ("value", Value::Number(*n)),
        ]),
        Expr::Str(s) => obj(&[
            ("tag", Value::Str("Str".into())),
            ("value", Value::Str(s.clone())),
        ]),
        Expr::Bool(b) => obj(&[
            ("tag", Value::Str("Bool".into())),
            ("value", Value::Bool(*b)),
        ]),
        Expr::Null => obj(&[("tag", Value::Str("Null".into()))]),
        Expr::Ident(name) => obj(&[
            ("tag", Value::Str("Ident".into())),
            ("name", Value::Str(name.clone())),
        ]),
        Expr::FieldAccess { object, field } => obj(&[
            ("tag", Value::Str("Field".into())),
            ("obj", expr_to_value(object)),
            ("field", Value::Str(field.clone())),
        ]),
        Expr::Index { object, index } => obj(&[
            ("tag", Value::Str("Index".into())),
            ("obj", expr_to_value(object)),
            ("idx", expr_to_value(index)),
        ]),
        Expr::Call { callee, args } => {
            if let Expr::FieldAccess { object, field } = callee.as_ref() {
                let arg_vals: Vec<Value> = args.iter().map(expr_to_value).collect();
                return obj(&[
                    ("tag", Value::Str("MethodCall".into())),
                    ("obj", expr_to_value(object)),
                    ("method", Value::Str(field.clone())),
                    ("args", Value::Array(arg_vals)),
                ]);
            }
            let arg_vals: Vec<Value> = args.iter().map(expr_to_value).collect();
            obj(&[
                ("tag", Value::Str("Call".into())),
                ("callee", expr_to_value(callee)),
                ("args", Value::Array(arg_vals)),
            ])
        }
        Expr::Array(items) => obj(&[
            ("tag", Value::Str("Array".into())),
            (
                "items",
                Value::Array(items.iter().map(expr_to_value).collect()),
            ),
        ]),
        Expr::Object(pairs) => {
            let pair_vals: Vec<Value> = pairs
                .iter()
                .map(|(k, v)| obj(&[("key", Value::Str(k.clone())), ("value", expr_to_value(v))]))
                .collect();
            obj(&[
                ("tag", Value::Str("Object".into())),
                ("pairs", Value::Array(pair_vals)),
            ])
        }
        Expr::BinOp { left, op, right } => {
            if matches!(op, BinOp::NullCoalesce) {
                return obj(&[
                    ("tag", Value::Str("NullCoal".into())),
                    ("left", expr_to_value(left)),
                    ("right", expr_to_value(right)),
                ]);
            }
            let op_str = match op {
                BinOp::Add | BinOp::Concat => "+",
                BinOp::Sub => "-",
                BinOp::Mul => "*",
                BinOp::Div => "/",
                BinOp::Mod => "%",
                BinOp::Eq => "==",
                BinOp::NotEq => "!=",
                BinOp::Lt => "<",
                BinOp::LtEq => "<=",
                BinOp::Gt => ">",
                BinOp::GtEq => ">=",
                BinOp::And => "and",
                BinOp::Or => "or",
                BinOp::Pipe => "|>",
                BinOp::NullCoalesce => "??",
            };
            obj(&[
                ("tag", Value::Str("BinOp".into())),
                ("op", Value::Str(op_str.into())),
                ("left", expr_to_value(left)),
                ("right", expr_to_value(right)),
            ])
        }
        Expr::Not(inner) => obj(&[
            ("tag", Value::Str("Unary".into())),
            ("op", Value::Str("not".into())),
            ("expr", expr_to_value(inner)),
        ]),
        Expr::Interpolated(parts) => {
            let part_vals: Vec<Value> = parts
                .iter()
                .map(|p| match p {
                    InterpolatedPart::Literal(s) => obj(&[
                        ("tag", Value::Str("Lit".into())),
                        ("v", Value::Str(s.clone())),
                    ]),
                    InterpolatedPart::Expr(e) => {
                        obj(&[("tag", Value::Str("Expr".into())), ("e", expr_to_value(e))])
                    }
                })
                .collect();
            obj(&[
                ("tag", Value::Str("Interp".into())),
                ("parts", Value::Array(part_vals)),
            ])
        }
        _ => obj(&[("tag", Value::Str("Null".into()))]),
    }
}

fn obj(pairs: &[(&str, Value)]) -> Value {
    let map: HashMap<String, Value> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    Value::Object(map)
}
