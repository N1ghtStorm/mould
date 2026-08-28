pub mod ast;
pub mod lexer;
pub mod parser;
pub mod runtime;

pub use ast::{
    BinaryExpression, BinaryOperator, Block, CallExpression, CallStatement, Expression,
    FieldAccess, Function, FunctionParameter, LetStatement, Program, ReturnStatement, Statement,
    StructDefinition, StructField, StructLiteral, StructLiteralField, Type,
};
pub use lexer::{LexError, Span};
pub use parser::{ParseError, parse_source};
pub use runtime::{RuntimeError, run_main};
