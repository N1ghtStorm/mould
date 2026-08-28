#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub functions: Vec<Function>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    pub body: Block,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub statements: Vec<Statement>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Statement {
    Let(LetStatement),
    Call(CallStatement),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LetStatement {
    pub name: String,
    pub ty: Type,
    pub value: Expression,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallStatement {
    pub name: String,
    pub arguments: Vec<Expression>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    I32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expression {
    Integer(i32),
    Variable(String),
}
