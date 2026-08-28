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
    Let,
    I32,
    Ident(String),
    Integer(String),
    LeftParen,
    RightParen,
    LeftBrace,
    RightBrace,
    Colon,
    Comma,
    Equal,
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
                ':' => self.push_single(TokenKind::Colon),
                ',' => self.push_single(TokenKind::Comma),
                '=' => self.push_single(TokenKind::Equal),
                ';' => self.push_single(TokenKind::Semicolon),
                ch if is_ident_start(ch) => self.lex_identifier(),
                ch if ch.is_ascii_digit() => self.lex_integer(),
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
        let kind = match text {
            "fn" => TokenKind::Fn,
            "let" => TokenKind::Let,
            "i32" => TokenKind::I32,
            _ => TokenKind::Ident(text.to_string()),
        };

        self.tokens.push(Token {
            kind,
            span: Span {
                start,
                end: self.position,
            },
        });
    }

    fn lex_integer(&mut self) {
        let start = self.position;
        self.bump();

        while let Some(ch) = self.peek() {
            if !ch.is_ascii_digit() {
                break;
            }
            self.bump();
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

    fn peek(&self) -> Option<char> {
        self.source[self.position..].chars().next()
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
