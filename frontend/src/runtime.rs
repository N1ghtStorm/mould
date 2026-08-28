use std::collections::HashMap;

use crate::{
    CallExpression, Expression, Function, Program,
    Statement::{self, Call, Let, Return},
    Type,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeError {
    pub message: String,
}

pub fn run_main(program: &Program) -> Result<String, RuntimeError> {
    let mut runtime = Runtime::new(program)?;
    runtime.call_main()?;
    Ok(runtime.output)
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

struct Runtime<'program> {
    functions: HashMap<String, &'program Function>,
    output: String,
}

impl<'program> Runtime<'program> {
    fn new(program: &'program Program) -> Result<Self, RuntimeError> {
        let mut functions = HashMap::new();

        for function in &program.functions {
            if functions.insert(function.name.clone(), function).is_some() {
                return Err(RuntimeError {
                    message: format!("function `{}` is already defined", function.name),
                });
            }
        }

        Ok(Self {
            functions,
            output: String::new(),
        })
    }

    fn call_main(&mut self) -> Result<Option<Value>, RuntimeError> {
        self.call_function("main", &[], &HashMap::new())
    }

    fn call_function(
        &mut self,
        name: &str,
        arguments: &[Expression],
        caller_variables: &HashMap<String, Value>,
    ) -> Result<Option<Value>, RuntimeError> {
        let function = *self.functions.get(name).ok_or_else(|| RuntimeError {
            message: format!("unknown function `{name}`"),
        })?;

        if function.parameters.len() != arguments.len() {
            return Err(RuntimeError {
                message: format!(
                    "function `{name}` expects {} argument(s)",
                    function.parameters.len()
                ),
            });
        }

        let mut variables = HashMap::new();

        for (parameter, argument) in function.parameters.iter().zip(arguments) {
            let value = self.evaluate_as(argument, parameter.ty, caller_variables)?;
            variables.insert(parameter.name.clone(), value);
        }

        let returned = self.run_function(function, &mut variables)?;

        match (function.return_type, returned) {
            (Some(_), Some(value)) => Ok(Some(value)),
            (Some(ty), None) => Err(RuntimeError {
                message: format!("function `{name}` must return `{}`", ty.name()),
            }),
            (None, Some(_)) => Err(RuntimeError {
                message: format!("function `{name}` cannot return a value"),
            }),
            (None, None) => Ok(None),
        }
    }

    fn run_function(
        &mut self,
        function: &Function,
        variables: &mut HashMap<String, Value>,
    ) -> Result<Option<Value>, RuntimeError> {
        for statement in &function.body.statements {
            if let Some(value) = self.run_statement(statement, function.return_type, variables)? {
                return Ok(Some(value));
            }
        }

        Ok(None)
    }

    fn run_statement(
        &mut self,
        statement: &Statement,
        return_type: Option<Type>,
        variables: &mut HashMap<String, Value>,
    ) -> Result<Option<Value>, RuntimeError> {
        match statement {
            Let(statement) => {
                let value = self.evaluate_as(&statement.value, statement.ty, variables)?;
                variables.insert(statement.name.clone(), value);
                Ok(None)
            }
            Call(statement) => {
                let call = CallExpression {
                    name: statement.name.clone(),
                    arguments: statement.arguments.clone(),
                };
                self.evaluate_call_statement(&call, variables)?;
                Ok(None)
            }
            Return(statement) => {
                let Some(ty) = return_type else {
                    return Err(RuntimeError {
                        message: "cannot return a value from function without return type"
                            .to_string(),
                    });
                };

                self.evaluate_as(&statement.value, ty, variables).map(Some)
            }
        }
    }

    fn evaluate(
        &mut self,
        expression: &Expression,
        variables: &HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        match expression {
            Expression::Integer(value) => integer_value(*value, Type::I32),
            Expression::Float(value) => float_value(value, Type::F64),
            Expression::Bool(value) => Ok(Value::Bool(*value)),
            Expression::Variable(name) => {
                variables.get(name).cloned().ok_or_else(|| RuntimeError {
                    message: format!("variable `{name}` not found"),
                })
            }
            Expression::Call(call) => self.evaluate_call_expression(call, variables),
        }
    }

    fn evaluate_as(
        &mut self,
        expression: &Expression,
        ty: Type,
        variables: &HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        match expression {
            Expression::Integer(value) => integer_value(*value, ty),
            Expression::Float(value) => float_value(value, ty),
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
                let value = variables.get(name).cloned().ok_or_else(|| RuntimeError {
                    message: format!("variable `{name}` not found"),
                })?;
                expect_type(value, ty)
            }
            Expression::Call(call) => {
                let value = self.evaluate_call_expression(call, variables)?;
                expect_type(value, ty)
            }
        }
    }

    fn evaluate_call_statement(
        &mut self,
        call: &CallExpression,
        variables: &HashMap<String, Value>,
    ) -> Result<(), RuntimeError> {
        if call.name == "println" {
            return self.evaluate_println(call, variables);
        }

        self.call_function(&call.name, &call.arguments, variables)?;
        Ok(())
    }

    fn evaluate_call_expression(
        &mut self,
        call: &CallExpression,
        variables: &HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if call.name == "println" {
            return Err(RuntimeError {
                message: "function `println` does not return a value".to_string(),
            });
        }

        self.call_function(&call.name, &call.arguments, variables)?
            .ok_or_else(|| RuntimeError {
                message: format!("function `{}` does not return a value", call.name),
            })
    }

    fn evaluate_println(
        &mut self,
        call: &CallExpression,
        variables: &HashMap<String, Value>,
    ) -> Result<(), RuntimeError> {
        if call.arguments.len() != 1 {
            return Err(RuntimeError {
                message: "function `println` expects 1 argument".to_string(),
            });
        }

        let value = self.evaluate(&call.arguments[0], variables)?;
        self.output.push_str(&value.printable());
        self.output.push('\n');
        Ok(())
    }
}

fn expect_type(value: Value, ty: Type) -> Result<Value, RuntimeError> {
    if value.ty() == ty {
        Ok(value)
    } else {
        Err(RuntimeError {
            message: format!(
                "cannot use `{}` value as `{}`",
                value.ty().name(),
                ty.name()
            ),
        })
    }
}

fn integer_value(value: u128, ty: Type) -> Result<Value, RuntimeError> {
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

fn float_value(value: &str, ty: Type) -> Result<Value, RuntimeError> {
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
    fn calls_function_with_params_and_return() {
        let program = parse_source(
            "fn sample(a: i32, b: bool) -> i32 { return a } fn main() { println(sample(7, true)) }",
        )
        .unwrap();

        assert_eq!(run_main(&program).unwrap(), "7\n");
    }

    #[test]
    fn calls_void_function_with_params() {
        let program = parse_source("fn log(a: i32) { println(a) } fn main() { log(8) }").unwrap();

        assert_eq!(run_main(&program).unwrap(), "8\n");
    }

    #[test]
    fn rejects_missing_return() {
        let program = parse_source("fn sample() -> i32 {} fn main() { sample() }").unwrap();
        let error = run_main(&program).unwrap_err();

        assert!(error.message.contains("must return `i32`"));
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
