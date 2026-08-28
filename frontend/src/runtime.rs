use std::collections::HashMap;

use crate::{
    Expression, Function, Program,
    Statement::{self, Call, Let},
    Type,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeError {
    pub message: String,
}

pub fn run_main(program: &Program) -> Result<String, RuntimeError> {
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .ok_or_else(|| RuntimeError {
            message: "function `main` not found".to_string(),
        })?;

    Runtime::new().run_function(main)
}

#[derive(Debug, Clone, PartialEq)]
enum Value {
    Integer { value: u128, ty: Type },
    Float { value: f64, ty: Type },
    Bool(bool),
}

impl Value {
    fn ty(&self) -> Type {
        match self {
            Self::Integer { ty, .. } | Self::Float { ty, .. } => *ty,
            Self::Bool(_) => Type::Bool,
        }
    }

    fn printable(&self) -> String {
        match self {
            Self::Integer { value, .. } => value.to_string(),
            Self::Float { value, .. } => value.to_string(),
            Self::Bool(value) => value.to_string(),
        }
    }
}

struct Runtime {
    variables: HashMap<String, Value>,
    output: String,
}

impl Runtime {
    fn new() -> Self {
        Self {
            variables: HashMap::new(),
            output: String::new(),
        }
    }

    fn run_function(mut self, function: &Function) -> Result<String, RuntimeError> {
        for statement in &function.body.statements {
            self.run_statement(statement)?;
        }

        Ok(self.output)
    }

    fn run_statement(&mut self, statement: &Statement) -> Result<(), RuntimeError> {
        match statement {
            Let(statement) => {
                let value = self.evaluate_as(&statement.value, statement.ty)?;
                self.variables.insert(statement.name.clone(), value);
                Ok(())
            }
            Call(statement) => {
                if statement.name != "println" {
                    return Err(RuntimeError {
                        message: format!("unknown function `{}`", statement.name),
                    });
                }

                if statement.arguments.len() != 1 {
                    return Err(RuntimeError {
                        message: "function `println` expects 1 argument".to_string(),
                    });
                }

                let value = self.evaluate(&statement.arguments[0])?;
                self.output.push_str(&value.printable());
                self.output.push('\n');
                Ok(())
            }
        }
    }

    fn evaluate(&self, expression: &Expression) -> Result<Value, RuntimeError> {
        match expression {
            Expression::Integer(value) => self.integer_value(*value, Type::I32),
            Expression::Float(value) => self.float_value(value, Type::F64),
            Expression::Bool(value) => Ok(Value::Bool(*value)),
            Expression::Variable(name) => {
                self.variables
                    .get(name)
                    .cloned()
                    .ok_or_else(|| RuntimeError {
                        message: format!("variable `{name}` not found"),
                    })
            }
        }
    }

    fn evaluate_as(&self, expression: &Expression, ty: Type) -> Result<Value, RuntimeError> {
        match expression {
            Expression::Integer(value) => self.integer_value(*value, ty),
            Expression::Float(value) => self.float_value(value, ty),
            Expression::Bool(value) => {
                if ty.is_bool() {
                    Ok(Value::Bool(*value))
                } else {
                    Err(RuntimeError {
                        message: format!("cannot assign bool literal to `{}`", ty.name()),
                    })
                }
            }
            Expression::Variable(name) => {
                let value = self
                    .variables
                    .get(name)
                    .cloned()
                    .ok_or_else(|| RuntimeError {
                        message: format!("variable `{name}` not found"),
                    })?;

                if value.ty() == ty {
                    Ok(value)
                } else {
                    Err(RuntimeError {
                        message: format!(
                            "cannot assign `{}` value to `{}`",
                            value.ty().name(),
                            ty.name()
                        ),
                    })
                }
            }
        }
    }

    fn integer_value(&self, value: u128, ty: Type) -> Result<Value, RuntimeError> {
        if !ty.is_integer() {
            return Err(RuntimeError {
                message: format!("cannot assign integer literal to `{}`", ty.name()),
            });
        }

        let max = ty.max_integer_value().expect("integer type has max value");

        if value > max {
            return Err(RuntimeError {
                message: format!("integer literal `{value}` does not fit in `{}`", ty.name()),
            });
        }

        Ok(Value::Integer { value, ty })
    }

    fn float_value(&self, value: &str, ty: Type) -> Result<Value, RuntimeError> {
        if !ty.is_float() {
            return Err(RuntimeError {
                message: format!("cannot assign float literal to `{}`", ty.name()),
            });
        }

        let parsed = value.parse::<f64>().map_err(|_| RuntimeError {
            message: format!("float literal `{value}` is invalid"),
        })?;

        Ok(Value::Float { value: parsed, ty })
    }
}

#[cfg(test)]
mod tests {
    use crate::{parse_source, run_main};

    #[test]
    fn println_outputs_number() {
        let program = parse_source("fn main() { println(1) }").unwrap();

        assert_eq!(run_main(&program).unwrap(), "1\n");
    }

    #[test]
    fn println_outputs_variable() {
        let program = parse_source("fn main() { let a: i32 = 1 println(a) }").unwrap();

        assert_eq!(run_main(&program).unwrap(), "1\n");
    }

    #[test]
    fn println_outputs_bool() {
        let program = parse_source("fn main() { let ready: bool = true println(ready) }").unwrap();

        assert_eq!(run_main(&program).unwrap(), "true\n");
    }

    #[test]
    fn println_outputs_float() {
        let program = parse_source("fn main() { let n: f64 = 1.5 println(n) }").unwrap();

        assert_eq!(run_main(&program).unwrap(), "1.5\n");
    }

    #[test]
    fn rejects_integer_out_of_range() {
        let program = parse_source("fn main() { let n: i8 = 128 }").unwrap();
        let error = run_main(&program).unwrap_err();

        assert!(error.message.contains("does not fit in `i8`"));
    }

    #[test]
    fn rejects_unknown_function() {
        let program = parse_source("fn main() { print(1) }").unwrap();
        let error = run_main(&program).unwrap_err();

        assert!(error.message.contains("unknown function `print`"));
    }

    #[test]
    fn rejects_missing_println_argument() {
        let program = parse_source("fn main() { println() }").unwrap();
        let error = run_main(&program).unwrap_err();

        assert!(error.message.contains("expects 1 argument"));
    }
}
