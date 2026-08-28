use std::collections::HashMap;

use crate::{
    Expression, Function, Program,
    Statement::{self, Call, Let},
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

struct Runtime {
    variables: HashMap<String, i32>,
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
                let value = self.evaluate(&statement.value)?;
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
                self.output.push_str(&format!("{value}\n"));
                Ok(())
            }
        }
    }

    fn evaluate(&self, expression: &Expression) -> Result<i32, RuntimeError> {
        match expression {
            Expression::Integer(value) => Ok(*value),
            Expression::Variable(name) => {
                self.variables
                    .get(name)
                    .copied()
                    .ok_or_else(|| RuntimeError {
                        message: format!("variable `{name}` not found"),
                    })
            }
        }
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
