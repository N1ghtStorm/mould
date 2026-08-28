use crate::{
    ast::{
        Block, CallExpression, CallStatement, Expression, Function, FunctionParameter,
        LetStatement, Program, ReturnStatement, Statement, Type,
    },
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
        let name = self.expect_identifier("function name")?;
        self.expect_simple(TokenKind::LeftParen, "`(`")?;
        let parameters = self.parse_parameters()?;
        self.expect_simple(TokenKind::RightParen, "`)`")?;
        let return_type = if self.eat(TokenKind::Arrow) {
            Some(self.parse_type()?)
        } else {
            None
        };
        let body = self.parse_block()?;

        Ok(Function {
            name,
            parameters,
            return_type,
            body,
        })
    }

    fn parse_parameters(&mut self) -> Result<Vec<FunctionParameter>, ParseError> {
        let mut parameters = Vec::new();

        if self.at(&TokenKind::RightParen) {
            return Ok(parameters);
        }

        loop {
            let name = self.expect_identifier("parameter name")?;
            self.expect_simple(TokenKind::Colon, "`:`")?;
            let ty = self.parse_type()?;
            parameters.push(FunctionParameter { name, ty });

            if !self.eat(TokenKind::Comma) {
                break;
            }
        }

        Ok(parameters)
    }

    fn parse_block(&mut self) -> Result<Block, ParseError> {
        self.expect_simple(TokenKind::LeftBrace, "`{`")?;

        let mut statements = Vec::new();

        while !self.at(&TokenKind::RightBrace) {
            statements.push(self.parse_statement()?);
        }

        self.expect_simple(TokenKind::RightBrace, "`}`")?;
        Ok(Block { statements })
    }

    fn parse_statement(&mut self) -> Result<Statement, ParseError> {
        match &self.current().kind {
            TokenKind::Let => self.parse_let_statement().map(Statement::Let),
            TokenKind::Ident(_) => self.parse_call_statement().map(Statement::Call),
            TokenKind::Return => self.parse_return_statement().map(Statement::Return),
            _ => Err(ParseError {
                message: format!(
                    "expected statement, found {}",
                    describe_kind(&self.current().kind)
                ),
                span: self.current().span,
            }),
        }
    }

    fn parse_let_statement(&mut self) -> Result<LetStatement, ParseError> {
        self.expect_simple(TokenKind::Let, "`let`")?;
        let name = self.expect_identifier("variable name")?;
        self.expect_simple(TokenKind::Colon, "`:`")?;
        let ty = self.parse_type()?;
        self.expect_simple(TokenKind::Equal, "`=`")?;
        let value = self.parse_expression()?;
        self.eat(TokenKind::Semicolon);

        Ok(LetStatement { name, ty, value })
    }

    fn parse_call_statement(&mut self) -> Result<CallStatement, ParseError> {
        let call = self.parse_call_expression()?;
        self.eat(TokenKind::Semicolon);

        Ok(CallStatement {
            name: call.name,
            arguments: call.arguments,
        })
    }

    fn parse_return_statement(&mut self) -> Result<ReturnStatement, ParseError> {
        self.expect_simple(TokenKind::Return, "`return`")?;
        let value = self.parse_expression()?;
        self.eat(TokenKind::Semicolon);

        Ok(ReturnStatement { value })
    }

    fn parse_call_expression(&mut self) -> Result<CallExpression, ParseError> {
        let name = self.expect_identifier("function name")?;
        self.expect_simple(TokenKind::LeftParen, "`(`")?;

        let mut arguments = Vec::new();

        if !self.at(&TokenKind::RightParen) {
            loop {
                arguments.push(self.parse_expression()?);

                if !self.eat(TokenKind::Comma) {
                    break;
                }
            }
        }

        self.expect_simple(TokenKind::RightParen, "`)`")?;

        Ok(CallExpression { name, arguments })
    }

    fn parse_type(&mut self) -> Result<Type, ParseError> {
        let token = self.current();

        match token.kind {
            TokenKind::PrimitiveType(ty) => {
                self.advance();
                Ok(ty)
            }
            _ => Err(ParseError {
                message: format!(
                    "expected primitive type, found {}",
                    describe_kind(&token.kind)
                ),
                span: token.span,
            }),
        }
    }

    fn parse_expression(&mut self) -> Result<Expression, ParseError> {
        let token = self.current().clone();

        match token.kind {
            TokenKind::Integer(value) => {
                let parsed = value.parse::<u128>().map_err(|_| ParseError {
                    message: format!("integer literal `{value}` does not fit in u128"),
                    span: token.span,
                })?;
                self.advance();
                Ok(Expression::Integer(parsed))
            }
            TokenKind::Float(value) => {
                value.parse::<f64>().map_err(|_| ParseError {
                    message: format!("float literal `{value}` is invalid"),
                    span: token.span,
                })?;
                let value = value.clone();
                self.advance();
                Ok(Expression::Float(value))
            }
            TokenKind::BoolLiteral(value) => {
                self.advance();
                Ok(Expression::Bool(value))
            }
            TokenKind::Ident(_) if self.next_at(&TokenKind::LeftParen) => {
                self.parse_call_expression().map(Expression::Call)
            }
            TokenKind::Ident(name) => {
                self.advance();
                Ok(Expression::Variable(name))
            }
            _ => Err(ParseError {
                message: format!("expected expression, found {}", describe_kind(&token.kind)),
                span: token.span,
            }),
        }
    }

    fn expect_identifier(&mut self, label: &str) -> Result<String, ParseError> {
        let token = self.current();

        match &token.kind {
            TokenKind::Ident(name) => {
                let name = name.clone();
                self.advance();
                Ok(name)
            }
            _ => Err(ParseError {
                message: format!("expected {label}, found {}", describe_kind(&token.kind)),
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

    fn eat(&mut self, kind: TokenKind) -> bool {
        if self.at(&kind) {
            self.advance();
            return true;
        }

        false
    }

    fn next_at(&self, kind: &TokenKind) -> bool {
        self.tokens
            .get(self.position + 1)
            .is_some_and(|token| token.kind == *kind)
    }
}

fn describe_kind(kind: &TokenKind) -> String {
    match kind {
        TokenKind::Fn => "`fn`".to_string(),
        TokenKind::Let => "`let`".to_string(),
        TokenKind::Return => "`return`".to_string(),
        TokenKind::PrimitiveType(ty) => format!("type `{}`", ty.name()),
        TokenKind::BoolLiteral(value) => format!("bool literal `{value}`"),
        TokenKind::Ident(name) => format!("identifier `{name}`"),
        TokenKind::Integer(value) => format!("integer literal `{value}`"),
        TokenKind::Float(value) => format!("float literal `{value}`"),
        TokenKind::LeftParen => "`(`".to_string(),
        TokenKind::RightParen => "`)`".to_string(),
        TokenKind::LeftBrace => "`{`".to_string(),
        TokenKind::RightBrace => "`}`".to_string(),
        TokenKind::Colon => "`:`".to_string(),
        TokenKind::Comma => "`,`".to_string(),
        TokenKind::Arrow => "`->`".to_string(),
        TokenKind::Equal => "`=`".to_string(),
        TokenKind::Semicolon => "`;`".to_string(),
        TokenKind::Eof => "end of file".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        Expression,
        Statement::{self},
        Type,
    };

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
        assert!(program.functions[0].body.statements.is_empty());
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
    fn parses_function_parameters_and_return_type() {
        let program = parse_source("fn sample(a: i32, b: bool) -> i32 { return a }").unwrap();
        let function = &program.functions[0];

        assert_eq!(function.name, "sample");
        assert_eq!(function.return_type, Some(Type::I32));
        assert_eq!(function.parameters.len(), 2);
        assert_eq!(function.parameters[0].name, "a");
        assert_eq!(function.parameters[0].ty, Type::I32);
        assert_eq!(function.parameters[1].name, "b");
        assert_eq!(function.parameters[1].ty, Type::Bool);
    }

    #[test]
    fn parses_call_expression() {
        let program = parse_source("fn main() { let a: i32 = sample(1, true) }").unwrap();
        let statement = &program.functions[0].body.statements[0];

        assert_eq!(
            statement,
            &Statement::Let(crate::LetStatement {
                name: "a".to_string(),
                ty: Type::I32,
                value: Expression::Call(crate::CallExpression {
                    name: "sample".to_string(),
                    arguments: vec![Expression::Integer(1), Expression::Bool(true)],
                }),
            })
        );
    }

    #[test]
    fn parses_return_statement() {
        let program = parse_source("fn sample() -> i32 { return 1 }").unwrap();
        let statement = &program.functions[0].body.statements[0];

        assert_eq!(
            statement,
            &Statement::Return(crate::ReturnStatement {
                value: Expression::Integer(1),
            })
        );
    }

    #[test]
    fn parses_i32_variable() {
        let program = parse_source("fn main() { let a: i32 = 1 }").unwrap();
        let statement = &program.functions[0].body.statements[0];

        assert_eq!(
            statement,
            &Statement::Let(crate::LetStatement {
                name: "a".to_string(),
                ty: Type::I32,
                value: Expression::Integer(1),
            })
        );
    }

    #[test]
    fn parses_all_primitive_types() {
        let type_names = Type::ALL.map(Type::name).join(", ");
        let source = format!(
            "fn main() {{ {} }}",
            Type::ALL
                .into_iter()
                .enumerate()
                .map(|(index, ty)| match ty {
                    Type::F16 | Type::F32 | Type::F64 => {
                        format!("let v{index}: {} = 1.5", ty.name())
                    }
                    Type::Bool => format!("let v{index}: bool = true"),
                    _ => format!("let v{index}: {} = 1", ty.name()),
                })
                .collect::<Vec<_>>()
                .join("; ")
        );

        let program =
            parse_source(&source).unwrap_or_else(|error| panic!("{type_names}: {error:?}"));

        assert_eq!(program.functions[0].body.statements.len(), Type::ALL.len());
    }

    #[test]
    fn parses_bool_variable() {
        let program = parse_source("fn main() { let ready: bool = true }").unwrap();
        let statement = &program.functions[0].body.statements[0];

        assert_eq!(
            statement,
            &Statement::Let(crate::LetStatement {
                name: "ready".to_string(),
                ty: Type::Bool,
                value: Expression::Bool(true),
            })
        );
    }

    #[test]
    fn parses_float_variable() {
        let program = parse_source("fn main() { let pi: f64 = 3.14 }").unwrap();
        let statement = &program.functions[0].body.statements[0];

        assert_eq!(
            statement,
            &Statement::Let(crate::LetStatement {
                name: "pi".to_string(),
                ty: Type::F64,
                value: Expression::Float("3.14".to_string()),
            })
        );
    }

    #[test]
    fn allows_semicolon_after_variable() {
        let program = parse_source("fn main() { let a: i32 = 1; }").unwrap();

        assert_eq!(program.functions[0].body.statements.len(), 1);
    }

    #[test]
    fn parses_println_number_call() {
        let program = parse_source("fn main() { println(1) }").unwrap();
        let statement = &program.functions[0].body.statements[0];

        assert_eq!(
            statement,
            &Statement::Call(crate::CallStatement {
                name: "println".to_string(),
                arguments: vec![Expression::Integer(1)],
            })
        );
    }

    #[test]
    fn parses_println_variable_call() {
        let program = parse_source("fn main() { println(a); }").unwrap();
        let statement = &program.functions[0].body.statements[0];

        assert_eq!(
            statement,
            &Statement::Call(crate::CallStatement {
                name: "println".to_string(),
                arguments: vec![Expression::Variable("a".to_string())],
            })
        );
    }

    #[test]
    fn rejects_unknown_type_for_now() {
        let error = parse_source("fn nope() { let value: str = 1 }").unwrap_err();

        assert!(error.message.contains("expected primitive type"));
    }

    #[test]
    fn rejects_unknown_statement_for_now() {
        let error = parse_source("fn nope() { value }").unwrap_err();

        assert!(error.message.contains("expected `(`"));
    }
}
