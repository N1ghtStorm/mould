use crate::{
    ast::{Block, Function, Program},
    lexer::{LexError, Span, Token, TokenKind, lex},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
}

impl From<LexError> for ParseError {
    fn from(error: LexError) -> Self {
        Self {
            message: error.message,
            span: error.span,
        }
    }
}

pub fn parse_source(source: &str) -> Result<Program, ParseError> {
    let tokens = lex(source)?;
    Parser::new(tokens).parse_program()
}

struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            position: 0,
        }
    }

    fn parse_program(&mut self) -> Result<Program, ParseError> {
        let mut functions = Vec::new();

        while !self.at(&TokenKind::Eof) {
            functions.push(self.parse_function()?);
        }

        Ok(Program { functions })
    }

    fn parse_function(&mut self) -> Result<Function, ParseError> {
        self.expect_simple(TokenKind::Fn, "`fn`")?;
        let name = self.expect_identifier()?;
        self.expect_simple(TokenKind::LeftParen, "`(`")?;
        self.expect_simple(TokenKind::RightParen, "`)`")?;
        let body = self.parse_block()?;

        Ok(Function { name, body })
    }

    fn parse_block(&mut self) -> Result<Block, ParseError> {
        self.expect_simple(TokenKind::LeftBrace, "`{`")?;
        self.expect_simple(TokenKind::RightBrace, "`}`")?;
        Ok(Block)
    }

    fn expect_identifier(&mut self) -> Result<String, ParseError> {
        let token = self.current();

        match &token.kind {
            TokenKind::Ident(name) => {
                let name = name.clone();
                self.advance();
                Ok(name)
            }
            _ => Err(ParseError {
                message: format!(
                    "expected function name, found {}",
                    describe_kind(&token.kind)
                ),
                span: token.span,
            }),
        }
    }

    fn expect_simple(&mut self, expected: TokenKind, label: &str) -> Result<(), ParseError> {
        let token = self.current();

        if token.kind == expected {
            self.advance();
            return Ok(());
        }

        Err(ParseError {
            message: format!("expected {label}, found {}", describe_kind(&token.kind)),
            span: token.span,
        })
    }

    fn at(&self, kind: &TokenKind) -> bool {
        self.current().kind == *kind
    }

    fn current(&self) -> &Token {
        &self.tokens[self.position]
    }

    fn advance(&mut self) {
        if !self.at(&TokenKind::Eof) {
            self.position += 1;
        }
    }
}

fn describe_kind(kind: &TokenKind) -> String {
    match kind {
        TokenKind::Fn => "`fn`".to_string(),
        TokenKind::Ident(name) => format!("identifier `{name}`"),
        TokenKind::LeftParen => "`(`".to_string(),
        TokenKind::RightParen => "`)`".to_string(),
        TokenKind::LeftBrace => "`{`".to_string(),
        TokenKind::RightBrace => "`}`".to_string(),
        TokenKind::Eof => "end of file".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_source;

    #[test]
    fn parses_empty_function() {
        let program = parse_source(
            r#"
            fn funcname() {
            }
            "#,
        )
        .unwrap();

        assert_eq!(program.functions.len(), 1);
        assert_eq!(program.functions[0].name, "funcname");
    }

    #[test]
    fn parses_multiple_functions() {
        let program = parse_source("fn first() {} fn second_2() {}").unwrap();

        let names: Vec<_> = program
            .functions
            .iter()
            .map(|function| function.name.as_str())
            .collect();

        assert_eq!(names, ["first", "second_2"]);
    }

    #[test]
    fn rejects_function_parameters_for_now() {
        let error = parse_source("fn nope(value) {}").unwrap_err();

        assert!(error.message.contains("expected `)`"));
    }

    #[test]
    fn rejects_non_empty_body_for_now() {
        let error = parse_source("fn nope() { value }").unwrap_err();

        assert!(error.message.contains("expected `}`"));
    }
}
