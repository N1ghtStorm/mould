use crate::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Fn,
    If,
    Else,
    Let,
    Loop,
    While,
    Break,
    Continue,
    Pub,
    Return,
    Struct,
    PrimitiveType(Type),
    BoolLiteral(bool),
    Ident(String),
    Integer(String),
    Float(String),
    LeftParen,
    RightParen,
    LeftBrace,
    RightBrace,
    Ampersand,
    DoubleAmpersand,
    Pipe,
    DoublePipe,
    Caret,
    Bang,
    BangEqual,
    Colon,
    Comma,
    Dot,
    Plus,
    Minus,
    Star,
    Slash,
    LeftShift,
    RightShift,
    Arrow,
    Equal,
    EqualEqual,
    Semicolon,
    Eof,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexError {
    pub message: String,
    pub span: Span,
}

pub fn lex(source: &str) -> Result<Vec<Token>, LexError> {
    let mut lexer = Lexer {
        source,
        position: 0,
        tokens: Vec::new(),
    };

    lexer.lex()?;
    Ok(lexer.tokens)
}

struct Lexer<'source> {
    source: &'source str,
    position: usize,
    tokens: Vec<Token>,
}

impl Lexer<'_> {
    fn lex(&mut self) -> Result<(), LexError> {
        while let Some(ch) = self.peek() {
            match ch {
                ch if ch.is_whitespace() => {
                    self.bump();
                }
                '(' => self.push_single(TokenKind::LeftParen),
                ')' => self.push_single(TokenKind::RightParen),
                '{' => self.push_single(TokenKind::LeftBrace),
                '}' => self.push_single(TokenKind::RightBrace),
                '&' if self.peek_next() == Some('&') => {
                    self.push_double(TokenKind::DoubleAmpersand);
                }
                '&' => self.push_single(TokenKind::Ampersand),
                '|' if self.peek_next() == Some('|') => {
                    self.push_double(TokenKind::DoublePipe);
                }
                '|' => self.push_single(TokenKind::Pipe),
                '^' => self.push_single(TokenKind::Caret),
                '!' if self.peek_next() == Some('=') => {
                    self.push_double(TokenKind::BangEqual);
                }
                '!' => self.push_single(TokenKind::Bang),
                ':' => self.push_single(TokenKind::Colon),
                ',' => self.push_single(TokenKind::Comma),
                '.' => self.push_single(TokenKind::Dot),
                '+' => self.push_single(TokenKind::Plus),
                '*' => self.push_single(TokenKind::Star),
                '/' => self.push_single(TokenKind::Slash),
                '<' if self.peek_next() == Some('<') => {
                    self.push_double(TokenKind::LeftShift);
                }
                '>' if self.peek_next() == Some('>') => {
                    self.push_double(TokenKind::RightShift);
                }
                '-' if self.peek_next() == Some('>') => {
                    self.push_double(TokenKind::Arrow);
                }
                '-' => self.push_single(TokenKind::Minus),
                '=' if self.peek_next() == Some('=') => {
                    self.push_double(TokenKind::EqualEqual);
                }
                '=' => self.push_single(TokenKind::Equal),
                ';' => self.push_single(TokenKind::Semicolon),
                ch if is_ident_start(ch) => self.lex_identifier(),
                ch if ch.is_ascii_digit() => self.lex_number(),
                _ => {
                    let start = self.position;
                    self.bump();
                    return Err(LexError {
                        message: format!("unexpected character `{ch}`"),
                        span: Span {
                            start,
                            end: self.position,
                        },
                    });
                }
            }
        }

        self.tokens.push(Token {
            kind: TokenKind::Eof,
            span: Span {
                start: self.position,
                end: self.position,
            },
        });

        Ok(())
    }

    fn lex_identifier(&mut self) {
        let start = self.position;
        self.bump();

        while let Some(ch) = self.peek() {
            if !is_ident_continue(ch) {
                break;
            }
            self.bump();
        }

        let text = &self.source[start..self.position];
        let kind = match (text, Type::from_name(text)) {
            ("fn", _) => TokenKind::Fn,
            ("if", _) => TokenKind::If,
            ("else", _) => TokenKind::Else,
            ("let", _) => TokenKind::Let,
            ("loop", _) => TokenKind::Loop,
            ("while", _) => TokenKind::While,
            ("break", _) => TokenKind::Break,
            ("continue", _) => TokenKind::Continue,
            ("pub", _) => TokenKind::Pub,
            ("return", _) => TokenKind::Return,
            ("struct", _) => TokenKind::Struct,
            ("true", _) => TokenKind::BoolLiteral(true),
            ("false", _) => TokenKind::BoolLiteral(false),
            (_, Some(ty)) => TokenKind::PrimitiveType(ty),
            (_, None) => TokenKind::Ident(text.to_string()),
        };

        self.tokens.push(Token {
            kind,
            span: Span {
                start,
                end: self.position,
            },
        });
    }

    fn lex_number(&mut self) {
        let start = self.position;
        self.bump();

        while let Some(ch) = self.peek() {
            if !ch.is_ascii_digit() {
                break;
            }
            self.bump();
        }

        if self.peek() == Some('.') && self.peek_next().is_some_and(|ch| ch.is_ascii_digit()) {
            self.bump();

            while let Some(ch) = self.peek() {
                if !ch.is_ascii_digit() {
                    break;
                }
                self.bump();
            }

            self.tokens.push(Token {
                kind: TokenKind::Float(self.source[start..self.position].to_string()),
                span: Span {
                    start,
                    end: self.position,
                },
            });
            return;
        }

        self.tokens.push(Token {
            kind: TokenKind::Integer(self.source[start..self.position].to_string()),
            span: Span {
                start,
                end: self.position,
            },
        });
    }

    fn push_single(&mut self, kind: TokenKind) {
        let start = self.position;
        self.bump();
        self.tokens.push(Token {
            kind,
            span: Span {
                start,
                end: self.position,
            },
        });
    }

    fn push_double(&mut self, kind: TokenKind) {
        let start = self.position;
        self.bump();
        self.bump();
        self.tokens.push(Token {
            kind,
            span: Span {
                start,
                end: self.position,
            },
        });
    }

    fn peek(&self) -> Option<char> {
        self.source[self.position..].chars().next()
    }

    fn peek_next(&self) -> Option<char> {
        let mut chars = self.source[self.position..].chars();
        chars.next()?;
        chars.next()
    }

    fn bump(&mut self) {
        if let Some(ch) = self.peek() {
            self.position += ch.len_utf8();
        }
    }
}

fn is_ident_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_ident_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}
