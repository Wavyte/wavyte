use crate::expression::ast::{BinaryOp, Expr, Lit, UnaryOp};
use crate::expression::error::ExprError;
use crate::expression::lexer::{Span, Token, TokenKind, lex};

pub(crate) fn parse_expr(src: &str) -> Result<Expr, ExprError> {
    let src = src.trim();
    let src = src.strip_prefix('=').unwrap_or(src);
    let tokens = lex(src)?;
    let mut p = Parser {
        src,
        tokens,
        pos: 0,
    };
    let expr = p.parse_or()?;
    p.expect(TokenKind::Eof)?;
    Ok(expr)
}

struct Parser<'a> {
    #[allow(dead_code)]
    src: &'a str,
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn bump(&mut self) -> &Token {
        let t = &self.tokens[self.pos];
        self.pos += 1;
        t
    }

    fn span(&self) -> Span {
        self.peek().span
    }

    fn expect(&mut self, kind: TokenKind) -> Result<(), ExprError> {
        if self.peek().kind == kind {
            self.bump();
            Ok(())
        } else {
            Err(ExprError::new(
                self.span().start,
                format!("expected {kind:?}, found {:?}", self.peek().kind),
            ))
        }
    }

    fn consume(&mut self, kind: TokenKind) -> bool {
        if self.peek().kind == kind {
            self.bump();
            true
        } else {
            false
        }
    }

    fn parse_or(&mut self) -> Result<Expr, ExprError> {
        let mut e = self.parse_and()?;
        while self.consume(TokenKind::OrOr) {
            let r = self.parse_and()?;
            e = Expr::Binary {
                op: BinaryOp::Or,
                left: Box::new(e),
                right: Box::new(r),
            };
        }
        Ok(e)
    }

    fn parse_and(&mut self) -> Result<Expr, ExprError> {
        let mut e = self.parse_equality()?;
        while self.consume(TokenKind::AndAnd) {
            let r = self.parse_equality()?;
            e = Expr::Binary {
                op: BinaryOp::And,
                left: Box::new(e),
                right: Box::new(r),
            };
        }
        Ok(e)
    }

    fn parse_equality(&mut self) -> Result<Expr, ExprError> {
        let mut e = self.parse_comparison()?;
        loop {
            if self.consume(TokenKind::EqEq) {
                let r = self.parse_comparison()?;
                e = Expr::Binary {
                    op: BinaryOp::Eq,
                    left: Box::new(e),
                    right: Box::new(r),
                };
            } else if self.consume(TokenKind::Ne) {
                let r = self.parse_comparison()?;
                e = Expr::Binary {
                    op: BinaryOp::Ne,
                    left: Box::new(e),
                    right: Box::new(r),
                };
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn parse_comparison(&mut self) -> Result<Expr, ExprError> {
        let mut e = self.parse_term()?;
        loop {
            let op = if self.consume(TokenKind::Lt) {
                Some(BinaryOp::Lt)
            } else if self.consume(TokenKind::Le) {
                Some(BinaryOp::Le)
            } else if self.consume(TokenKind::Gt) {
                Some(BinaryOp::Gt)
            } else if self.consume(TokenKind::Ge) {
                Some(BinaryOp::Ge)
            } else {
                None
            };
            if let Some(op) = op {
                let r = self.parse_term()?;
                e = Expr::Binary {
                    op,
                    left: Box::new(e),
                    right: Box::new(r),
                };
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn parse_term(&mut self) -> Result<Expr, ExprError> {
        let mut e = self.parse_factor()?;
        loop {
            if self.consume(TokenKind::Plus) {
                let r = self.parse_factor()?;
                e = Expr::Binary {
                    op: BinaryOp::Add,
                    left: Box::new(e),
                    right: Box::new(r),
                };
            } else if self.consume(TokenKind::Minus) {
                let r = self.parse_factor()?;
                e = Expr::Binary {
                    op: BinaryOp::Sub,
                    left: Box::new(e),
                    right: Box::new(r),
                };
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn parse_factor(&mut self) -> Result<Expr, ExprError> {
        let mut e = self.parse_unary()?;
        loop {
            if self.consume(TokenKind::Star) {
                let r = self.parse_unary()?;
                e = Expr::Binary {
                    op: BinaryOp::Mul,
                    left: Box::new(e),
                    right: Box::new(r),
                };
            } else if self.consume(TokenKind::Slash) {
                let r = self.parse_unary()?;
                e = Expr::Binary {
                    op: BinaryOp::Div,
                    left: Box::new(e),
                    right: Box::new(r),
                };
            } else if self.consume(TokenKind::Percent) {
                let r = self.parse_unary()?;
                e = Expr::Binary {
                    op: BinaryOp::Mod,
                    left: Box::new(e),
                    right: Box::new(r),
                };
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn parse_unary(&mut self) -> Result<Expr, ExprError> {
        if self.consume(TokenKind::Minus) {
            let e = self.parse_unary()?;
            return Ok(Expr::Unary {
                op: UnaryOp::Neg,
                expr: Box::new(e),
            });
        }
        if self.consume(TokenKind::Bang) {
            let e = self.parse_unary()?;
            return Ok(Expr::Unary {
                op: UnaryOp::Not,
                expr: Box::new(e),
            });
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr, ExprError> {
        let mut e = self.parse_primary()?;

        loop {
            if self.consume(TokenKind::Dot) {
                let t = self.bump().clone();
                let name = match t.kind {
                    TokenKind::Ident(s) => s,
                    other => {
                        return Err(ExprError::new(
                            t.span.start,
                            format!("expected ident after '.', found {other:?}"),
                        ));
                    }
                };
                e = append_path(e, name)?;
                continue;
            }

            if self.consume(TokenKind::LParen) {
                let args = self.parse_args()?;
                let func = match e {
                    Expr::Path(mut p) if p.len() == 1 => p.pop().unwrap(),
                    Expr::Lit(_)
                    | Expr::Unary { .. }
                    | Expr::Binary { .. }
                    | Expr::Call { .. }
                    | Expr::Prop(_)
                    | Expr::Var(_)
                    | Expr::Time(_) => {
                        return Err(ExprError::new(
                            self.span().start,
                            "call target must be an identifier",
                        ));
                    }
                    Expr::Path(p) => {
                        return Err(ExprError::new(
                            self.span().start,
                            format!("call target must be a single identifier, got path {:?}", p),
                        ));
                    }
                };
                e = Expr::Call { func, args };
                continue;
            }

            break;
        }

        Ok(e)
    }

    fn parse_args(&mut self) -> Result<Vec<Expr>, ExprError> {
        let mut args = Vec::new();
        if self.consume(TokenKind::RParen) {
            return Ok(args);
        }
        loop {
            args.push(self.parse_or()?);
            if self.consume(TokenKind::Comma) {
                continue;
            }
            self.expect(TokenKind::RParen)?;
            return Ok(args);
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, ExprError> {
        let t = self.bump().clone();
        match t.kind {
            TokenKind::Number(v) => Ok(Expr::Lit(Lit::F64(v))),
            TokenKind::True => Ok(Expr::Lit(Lit::Bool(true))),
            TokenKind::False => Ok(Expr::Lit(Lit::Bool(false))),
            TokenKind::Ident(s) => Ok(Expr::Path(vec![s])),
            TokenKind::LParen => {
                let e = self.parse_or()?;
                self.expect(TokenKind::RParen)?;
                Ok(e)
            }
            other => Err(ExprError::new(
                t.span.start,
                format!("unexpected token {other:?}"),
            )),
        }
    }
}

fn append_path(base: Expr, segment: String) -> Result<Expr, ExprError> {
    match base {
        Expr::Path(mut v) => {
            v.push(segment);
            Ok(Expr::Path(v))
        }
        _ => Err(ExprError::new(
            0,
            "member access base must be an identifier path",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expression::ast::Expr;

    #[test]
    fn parses_arithmetic_precedence() {
        let e = parse_expr("=1+2*3").unwrap();
        match e {
            Expr::Binary {
                op: BinaryOp::Add, ..
            } => {}
            other => panic!("unexpected ast: {other:?}"),
        }
    }

    #[test]
    fn parses_paths() {
        let e = parse_expr("nodes.title.transform.translate.x").unwrap();
        assert_eq!(
            e,
            Expr::Path(vec![
                "nodes".to_owned(),
                "title".to_owned(),
                "transform".to_owned(),
                "translate".to_owned(),
                "x".to_owned(),
            ])
        );
    }

    #[test]
    fn parses_calls() {
        let e = parse_expr("min(1,2)").unwrap();
        match e {
            Expr::Call { func, args } => {
                assert_eq!(func, "min");
                assert_eq!(args.len(), 2);
            }
            other => panic!("unexpected ast: {other:?}"),
        }
    }
}
