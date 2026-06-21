//! `calc.eval` — exact arithmetic expression evaluation.
//!
//! Chapter Abacus (AB.1). The first of the pure-compute utility
//! tools and the one that carries the chapter's governance spine:
//! a new `calc.eval` capability base in
//! [`aivyx_capability::KNOWN_BASES`], gated at the **SemiTrusted**
//! ceiling rather than the toolkit's usual Trusted-only default —
//! a calculator touches no network, no filesystem, and no
//! operator data, so it is safe below the Trusted tier (see
//! `docs/ABACUS.md` §2).
//!
//! ## Why a tool at all
//!
//! A language model is structurally unreliable at arithmetic: it
//! predicts plausible digits rather than computing them. Handing
//! the expression to a tiny, exact, audited evaluator turns a
//! confident guess into a correct answer.
//!
//! ## Tool surface
//!
//! - `calc.eval` — `{expr: string (required)}` →
//!   `{expr (echoed), result: number}`. The evaluator supports
//!   `+ - * / % ^`, parentheses, unary `+`/`-`, and a small fixed
//!   function whitelist (`sqrt`, `abs`, `round`, `floor`, `ceil`,
//!   `min`, `max`). No variables, no user-defined functions — that
//!   is a REPL, not a tool. A non-finite result (`NaN`/`±inf`, e.g.
//!   division by zero or `sqrt(-1)`) is a tool error, not a
//!   `result`.
//!
//! ## Dependency note (OQ-3, resolved)
//!
//! The evaluator is **hand-rolled** (recursive descent over a
//! hand-written tokenizer) — zero new dependencies, fully auditable,
//! and the function set is small enough that a general expression
//! crate would be more surface than it saves.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{
    AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification,
};

/// `calc.eval` — evaluate an arithmetic expression.
pub struct CalcEval {
    id: ToolId,
    schema: Value,
}

impl Default for CalcEval {
    fn default() -> Self {
        Self::new()
    }
}

impl CalcEval {
    pub fn new() -> Self {
        Self {
            id: ToolId::new(),
            schema: eval_schema(),
        }
    }
}

#[async_trait]
impl Tool for CalcEval {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "calc.eval"
    }
    fn description(&self) -> &str {
        "Evaluate an arithmetic expression exactly. Input: \
         `{expr: string (required)}`. Supports `+ - * / % ^`, \
         parentheses, unary `+`/`-`, and the functions `sqrt`, \
         `abs`, `round`, `floor`, `ceil`, `min`, `max` (the last \
         two take two or more comma-separated arguments). No \
         variables or user-defined functions. Returns `{expr, \
         result}`. A non-finite result (division by zero, \
         `sqrt(-1)`, …) is an error, not a result. Prefer this \
         over computing arithmetic yourself. Scope: `calc.eval`."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("calc.eval")
            .expect("calc.eval must parse — it is in KNOWN_BASES from Chapter Abacus")
    }
    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let expr = match required_string(&input, "expr") {
            Ok(s) => s,
            Err(e) => return failed(self.id, format!("calc.eval: {e}")),
        };
        match evaluate(&expr) {
            Ok(result) => ToolOutcome::Completed {
                output: json!({ "expr": expr, "result": result }),
                // Pure computation; there is nothing external to verify.
                verified: Verification::NotApplicable,
            },
            Err(e) => failed(self.id, format!("calc.eval: {e}")),
        }
    }
}

fn eval_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "expr": {
                "type": "string",
                "minLength": 1,
                "description": "Arithmetic expression, e.g. `(18.5/100)*2140` \
                                or `max(3, sqrt(16), 2^3)`."
            }
        },
        "required": ["expr"],
        "additionalProperties": false
    })
}

fn required_string(input: &Value, field: &str) -> Result<String, String> {
    let s = input
        .get(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("input must include an `{field}` string field"))?;
    if s.trim().is_empty() {
        return Err(format!("`{field}` must not be empty"));
    }
    Ok(s.to_string())
}

fn failed(id: ToolId, detail: String) -> ToolOutcome {
    ToolOutcome::Failed(AivyxError::Tool { tool: id, detail })
}

// =====================================================================
// The evaluator — hand-rolled recursive descent, zero dependencies.
//
// Grammar (precedence low → high). Unary minus binds *looser* than
// `^`, so `-2^2 == -(2^2) == -4` (the conventional reading), while
// the exponent may itself be unary so `2^-3` works:
//   expr   := term  (('+' | '-') term)*
//   term   := unary (('*' | '/' | '%') unary)*
//   unary  := ('+' | '-') unary | power
//   power  := primary ('^' unary)?          // right-associative
//   primary:= number | '(' expr ')' | ident '(' args ')'
//   args   := expr (',' expr)*
// =====================================================================

/// Evaluate an arithmetic expression to a finite `f64`.
///
/// Returns a human-readable error string on a parse error, an
/// unknown function, a wrong argument count, or a non-finite
/// result (`NaN`/`±inf`).
pub fn evaluate(expr: &str) -> Result<f64, String> {
    let tokens = tokenize(expr)?;
    let mut parser = Parser { tokens, pos: 0 };
    let value = parser.parse_expr()?;
    if parser.pos != parser.tokens.len() {
        return Err(format!(
            "unexpected trailing input near token {}",
            parser.pos + 1
        ));
    }
    if !value.is_finite() {
        return Err(
            "result is not a finite number (division by zero or out-of-domain function?)"
                .to_string(),
        );
    }
    Ok(value)
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    LParen,
    RParen,
    Comma,
}

fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            c if c.is_whitespace() => i += 1,
            '+' => {
                tokens.push(Token::Plus);
                i += 1;
            }
            '-' => {
                tokens.push(Token::Minus);
                i += 1;
            }
            '*' => {
                tokens.push(Token::Star);
                i += 1;
            }
            '/' => {
                tokens.push(Token::Slash);
                i += 1;
            }
            '%' => {
                tokens.push(Token::Percent);
                i += 1;
            }
            '^' => {
                tokens.push(Token::Caret);
                i += 1;
            }
            '(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            ',' => {
                tokens.push(Token::Comma);
                i += 1;
            }
            c if c.is_ascii_digit() || c == '.' => {
                let start = i;
                while i < chars.len()
                    && (chars[i].is_ascii_digit() || chars[i] == '.')
                {
                    i += 1;
                }
                // Optional scientific exponent: e / E [+/-] digits.
                if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                    let mut j = i + 1;
                    if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
                        j += 1;
                    }
                    if j < chars.len() && chars[j].is_ascii_digit() {
                        while j < chars.len() && chars[j].is_ascii_digit() {
                            j += 1;
                        }
                        i = j;
                    }
                }
                let literal: String = chars[start..i].iter().collect();
                let n: f64 = literal
                    .parse()
                    .map_err(|_| format!("invalid number literal `{literal}`"))?;
                tokens.push(Token::Number(n));
            }
            c if c.is_ascii_alphabetic() => {
                let start = i;
                while i < chars.len() && chars[i].is_ascii_alphabetic() {
                    i += 1;
                }
                let ident: String = chars[start..i].iter().collect();
                tokens.push(Token::Ident(ident.to_ascii_lowercase()));
            }
            other => return Err(format!("unexpected character `{other}`")),
        }
    }
    if tokens.is_empty() {
        return Err("empty expression".to_string());
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn parse_expr(&mut self) -> Result<f64, String> {
        let mut left = self.parse_term()?;
        while let Some(tok) = self.peek() {
            match tok {
                Token::Plus => {
                    self.pos += 1;
                    left += self.parse_term()?;
                }
                Token::Minus => {
                    self.pos += 1;
                    left -= self.parse_term()?;
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_term(&mut self) -> Result<f64, String> {
        let mut left = self.parse_unary()?;
        while let Some(tok) = self.peek() {
            match tok {
                Token::Star => {
                    self.pos += 1;
                    left *= self.parse_unary()?;
                }
                Token::Slash => {
                    self.pos += 1;
                    left /= self.parse_unary()?;
                }
                Token::Percent => {
                    self.pos += 1;
                    left %= self.parse_unary()?;
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<f64, String> {
        match self.peek() {
            Some(Token::Minus) => {
                self.pos += 1;
                Ok(-self.parse_unary()?)
            }
            Some(Token::Plus) => {
                self.pos += 1;
                self.parse_unary()
            }
            _ => self.parse_power(),
        }
    }

    fn parse_power(&mut self) -> Result<f64, String> {
        let base = self.parse_primary()?;
        if let Some(Token::Caret) = self.peek() {
            self.pos += 1;
            // Right-associative; exponent may be unary (`2^-3`).
            let exp = self.parse_unary()?;
            Ok(base.powf(exp))
        } else {
            Ok(base)
        }
    }

    fn parse_primary(&mut self) -> Result<f64, String> {
        match self.peek().cloned() {
            Some(Token::Number(n)) => {
                self.pos += 1;
                Ok(n)
            }
            Some(Token::LParen) => {
                self.pos += 1;
                let value = self.parse_expr()?;
                self.expect(Token::RParen, ")")?;
                Ok(value)
            }
            Some(Token::Ident(name)) => {
                self.pos += 1;
                self.expect(Token::LParen, "( after function name")?;
                let mut args = vec![self.parse_expr()?];
                while let Some(Token::Comma) = self.peek() {
                    self.pos += 1;
                    args.push(self.parse_expr()?);
                }
                self.expect(Token::RParen, ")")?;
                apply_function(&name, &args)
            }
            Some(other) => Err(format!("unexpected token `{other:?}`")),
            None => Err("unexpected end of expression".to_string()),
        }
    }

    fn expect(&mut self, want: Token, label: &str) -> Result<(), String> {
        match self.peek() {
            Some(tok) if *tok == want => {
                self.pos += 1;
                Ok(())
            }
            Some(other) => Err(format!("expected `{label}`, found `{other:?}`")),
            None => Err(format!("expected `{label}`, found end of expression")),
        }
    }
}

/// The fixed function whitelist (OQ-4). Small and auditable; grow on
/// demand, never with user-defined functions or variables.
fn apply_function(name: &str, args: &[f64]) -> Result<f64, String> {
    let unary = |f: fn(f64) -> f64| -> Result<f64, String> {
        if args.len() != 1 {
            Err(format!("`{name}` takes exactly 1 argument, got {}", args.len()))
        } else {
            Ok(f(args[0]))
        }
    };
    match name {
        "sqrt" => unary(f64::sqrt),
        "abs" => unary(f64::abs),
        "round" => unary(f64::round),
        "floor" => unary(f64::floor),
        "ceil" => unary(f64::ceil),
        "min" | "max" => {
            if args.len() < 2 {
                return Err(format!(
                    "`{name}` takes at least 2 arguments, got {}",
                    args.len()
                ));
            }
            let fold = if name == "min" { f64::min } else { f64::max };
            Ok(args.iter().copied().reduce(fold).expect("len >= 2"))
        }
        other => Err(format!("unknown function `{other}`")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(s: &str) -> f64 {
        evaluate(s).unwrap_or_else(|e| panic!("evaluate({s:?}) errored: {e}"))
    }

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    // ---- arithmetic & precedence --------------------------------

    #[test]
    fn basic_arithmetic() {
        assert_eq!(ev("1 + 2"), 3.0);
        assert_eq!(ev("10 - 4 - 3"), 3.0); // left-assoc
        assert_eq!(ev("2 * 3 * 4"), 24.0);
    }

    #[test]
    fn precedence_mul_over_add() {
        assert_eq!(ev("2 + 3 * 4"), 14.0);
        assert_eq!(ev("(2 + 3) * 4"), 20.0);
    }

    #[test]
    fn division_and_modulo() {
        assert_eq!(ev("7 / 2"), 3.5);
        assert_eq!(ev("7 % 3"), 1.0);
    }

    #[test]
    fn power_is_right_associative() {
        assert_eq!(ev("2 ^ 3"), 8.0);
        assert_eq!(ev("2 ^ 3 ^ 2"), 512.0); // 2^(3^2), not (2^3)^2=64
    }

    #[test]
    fn unary_minus_binds_looser_than_power() {
        // Conventional reading: -2^2 == -(2^2) == -4.
        assert_eq!(ev("-2^2"), -4.0);
        assert_eq!(ev("-(2^2)"), -4.0);
        assert_eq!(ev("(-2)^2"), 4.0);
        // Exponent may itself be unary.
        assert_eq!(ev("2^-3"), 0.125);
    }

    #[test]
    fn unary_minus_and_plus() {
        assert_eq!(ev("-5"), -5.0);
        assert_eq!(ev("--5"), 5.0);
        assert_eq!(ev("+-+3"), -3.0);
        assert_eq!(ev("3 - -2"), 5.0);
    }

    #[test]
    fn decimals_and_scientific() {
        assert!(approx(ev("0.1 + 0.2"), 0.3));
        assert_eq!(ev("1e3"), 1000.0);
        assert_eq!(ev("1.5e2"), 150.0);
        assert_eq!(ev("2E-1"), 0.2);
    }

    #[test]
    fn real_world_percentage() {
        // 18.5% of 2140
        assert!(approx(ev("(18.5/100)*2140"), 395.9));
    }

    // ---- functions ----------------------------------------------

    #[test]
    fn functions_unary() {
        assert_eq!(ev("sqrt(16)"), 4.0);
        assert_eq!(ev("abs(-7)"), 7.0);
        assert_eq!(ev("round(2.6)"), 3.0);
        assert_eq!(ev("floor(2.9)"), 2.0);
        assert_eq!(ev("ceil(2.1)"), 3.0);
    }

    #[test]
    fn functions_min_max_variadic() {
        assert_eq!(ev("min(3, 1, 2)"), 1.0);
        assert_eq!(ev("max(3, sqrt(16), 2^3)"), 8.0);
    }

    #[test]
    fn functions_case_insensitive() {
        assert_eq!(ev("SQRT(9)"), 3.0);
        assert_eq!(ev("Max(1, 2)"), 2.0);
    }

    #[test]
    fn nested_expression() {
        assert_eq!(ev("max(1, 2) + min(10, 3) * 2"), 8.0);
    }

    // ---- errors -------------------------------------------------

    #[test]
    fn error_empty() {
        assert!(evaluate("").is_err());
        assert!(evaluate("   ").is_err());
    }

    #[test]
    fn error_unknown_function() {
        let e = evaluate("tan(1)").unwrap_err();
        assert!(e.contains("unknown function"), "got: {e}");
    }

    #[test]
    fn error_wrong_arity() {
        assert!(evaluate("sqrt(1, 2)").unwrap_err().contains("exactly 1"));
        assert!(evaluate("min(1)").unwrap_err().contains("at least 2"));
    }

    #[test]
    fn error_unbalanced_parens() {
        assert!(evaluate("(1 + 2").is_err());
        assert!(evaluate("1 + 2)").is_err());
    }

    #[test]
    fn error_trailing_garbage() {
        assert!(evaluate("1 2").is_err());
        assert!(evaluate("1 +").is_err());
    }

    #[test]
    fn error_bad_character() {
        assert!(evaluate("1 & 2").is_err());
    }

    #[test]
    fn error_non_finite_is_rejected() {
        // Division by zero → inf → tool error, not a result.
        assert!(evaluate("1/0").unwrap_err().contains("finite"));
        // sqrt of a negative → NaN → tool error.
        assert!(evaluate("sqrt(-1)").unwrap_err().contains("finite"));
    }

    // ---- tool wiring --------------------------------------------

    #[test]
    fn tool_metadata_is_sound() {
        let tool = CalcEval::new();
        assert_eq!(tool.name(), "calc.eval");
        assert!(!tool.description().is_empty());
        assert_eq!(tool.input_schema()["type"], "object");
        assert_eq!(
            tool.required_scope(&json!({"expr": "1+1"})),
            Scope::parse("calc.eval").unwrap()
        );
    }
}
