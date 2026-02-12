use crate::expression::error::ExprError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Span {
    pub(crate) start: usize,
    pub(crate) end: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Token {
    pub(crate) kind: TokenKind,
    pub(crate) span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TokenKind {
    Ident(String),
    Number(f64),
    True,
    False,

    LParen,
    RParen,
    Comma,
    Dot,

    Plus,
    Minus,
    Star,
    Slash,
    Percent,

    Bang,

    EqEq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,

    AndAnd,
    OrOr,

    Question,
    Colon,

    Eof,
}

pub(crate) fn lex(input: &str) -> Result<Vec<Token>, ExprError> {
    let mut out = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0usize;

    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_whitespace() {
            i += 1;
            continue;
        }

        let start = i;

        // Number: [0-9]+(.[0-9]+)?([eE][+-]?[0-9]+)? or .[0-9]+([eE][+-]?[0-9]+)?
        if c.is_ascii_digit()
            || (c == '.' && i + 1 < bytes.len() && (bytes[i + 1] as char).is_ascii_digit())
        {
            // integer part
            if c == '.' {
                i += 1;
            } else {
                while i < bytes.len() && (bytes[i] as char).is_ascii_digit() {
                    i += 1;
                }
                // fractional part
                if i < bytes.len()
                    && (bytes[i] as char) == '.'
                    && i + 1 < bytes.len()
                    && (bytes[i + 1] as char).is_ascii_digit()
                {
                    i += 1;
                }
            }

            while i < bytes.len() && (bytes[i] as char).is_ascii_digit() {
                i += 1;
            }

            // exponent
            if i < bytes.len() && matches!(bytes[i] as char, 'e' | 'E') {
                let e_pos = i;
                i += 1;
                if i < bytes.len() && matches!(bytes[i] as char, '+' | '-') {
                    i += 1;
                }
                let exp_start = i;
                while i < bytes.len() && (bytes[i] as char).is_ascii_digit() {
                    i += 1;
                }
                if exp_start == i {
                    return Err(ExprError::new(
                        e_pos,
                        "invalid number exponent (expected digits)",
                    ));
                }
            }

            let s = &input[start..i];
            let v: f64 = s
                .parse()
                .map_err(|_| ExprError::new(start, "invalid number"))?;
            out.push(Token {
                kind: TokenKind::Number(v),
                span: Span { start, end: i },
            });
            continue;
        }

        // Ident
        if c.is_ascii_alphabetic() || c == '_' {
            i += 1;
            while i < bytes.len() {
                let ch = bytes[i] as char;
                if ch.is_ascii_alphanumeric() || ch == '_' {
                    i += 1;
                } else {
                    break;
                }
            }
            let s = &input[start..i];
            let kind = match s {
                "true" => TokenKind::True,
                "false" => TokenKind::False,
                _ => TokenKind::Ident(s.to_owned()),
            };
            out.push(Token {
                kind,
                span: Span { start, end: i },
            });
            continue;
        }

        // Two-char operators
        if i + 1 < bytes.len() {
            let two = &input[i..i + 2];
            let kind = match two {
                "&&" => Some(TokenKind::AndAnd),
                "||" => Some(TokenKind::OrOr),
                "==" => Some(TokenKind::EqEq),
                "!=" => Some(TokenKind::Ne),
                "<=" => Some(TokenKind::Le),
                ">=" => Some(TokenKind::Ge),
                _ => None,
            };
            if let Some(kind) = kind {
                i += 2;
                out.push(Token {
                    kind,
                    span: Span { start, end: i },
                });
                continue;
            }
        }

        // Single-char tokens
        let kind = match c {
            '(' => TokenKind::LParen,
            ')' => TokenKind::RParen,
            ',' => TokenKind::Comma,
            '.' => TokenKind::Dot,
            '+' => TokenKind::Plus,
            '-' => TokenKind::Minus,
            '*' => TokenKind::Star,
            '/' => TokenKind::Slash,
            '%' => TokenKind::Percent,
            '!' => TokenKind::Bang,
            '<' => TokenKind::Lt,
            '>' => TokenKind::Gt,
            '?' => TokenKind::Question,
            ':' => TokenKind::Colon,
            _ => {
                return Err(ExprError::new(start, format!("unexpected character '{c}'")));
            }
        };
        i += 1;
        out.push(Token {
            kind,
            span: Span { start, end: i },
        });
    }

    out.push(Token {
        kind: TokenKind::Eof,
        span: Span {
            start: input.len(),
            end: input.len(),
        },
    });

    Ok(out)
}
