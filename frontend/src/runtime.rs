use std::collections::HashMap;

use crate::{
    CallExpression, Expression, FieldAccess, Function, Program,
    Statement::{self, Call, Let, Return},
    StructDefinition, StructLiteral, Type,
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
    Integer {
        value: u128,
        ty: Type,
    },
    Float {
        value: f64,
        ty: Type,
    },
    Bool(bool),
    Struct {
        name: String,
        fields: Vec<(String, Value)>,
    },
    Pointer(Box<Value>),
}

impl Value {
    fn ty(&self) -> Type {
        match self {
            Self::Integer { ty, .. } | Self::Float { ty, .. } => ty.clone(),
            Self::Bool(_) => Type::Bool,
            Self::Struct { name, .. } => Type::Struct(name.clone()),
            Self::Pointer(value) => Type::Pointer(Box::new(value.ty())),
        }
    }

    fn printable(&self) -> String {
        match self {
            Self::Integer { value, .. } => value.to_string(),
            Self::Float { value, .. } => value.to_string(),
            Self::Bool(value) => value.to_string(),
            Self::Struct { name, fields } => {
                let fields = fields
                    .iter()
                    .map(|(name, value)| format!("{name}: {}", value.printable()))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{name} {{ {fields} }}")
            }
            Self::Pointer(value) => format!("&{}", value.printable()),
        }
    }
}

struct Runtime<'program> {
    structs: HashMap<String, &'program StructDefinition>,
    functions: HashMap<String, &'program Function>,
    output: String,
}

impl<'program> Runtime<'program> {
    fn new(program: &'program Program) -> Result<Self, RuntimeError> {
        let mut structs = HashMap::new();

        for structure in &program.structs {
            if Type::from_name(&structure.name).is_some() {
                return Err(RuntimeError {
                    message: format!("cannot define primitive type `{}`", structure.name),
                });
            }

            if structs.insert(structure.name.clone(), structure).is_some() {
                return Err(RuntimeError {
                    message: format!("struct `{}` is already defined", structure.name),
                });
            }

            let mut fields = HashMap::new();
            for field in &structure.fields {
                if fields.insert(field.name.clone(), ()).is_some() {
                    return Err(RuntimeError {
                        message: format!(
                            "field `{}` is already defined in struct `{}`",
                            field.name, structure.name
                        ),
                    });
                }
            }
        }

        let mut functions = HashMap::new();

        for function in &program.functions {
            if functions.insert(function.name.clone(), function).is_some() {
                return Err(RuntimeError {
                    message: format!("function `{}` is already defined", function.name),
                });
            }
        }

        for structure in &program.structs {
            for field in &structure.fields {
                validate_type(&field.ty, &structs)?;
            }
        }

        for function in &program.functions {
            for parameter in &function.parameters {
                validate_type(&parameter.ty, &structs)?;
            }

            if let Some(return_type) = &function.return_type {
                validate_type(return_type, &structs)?;
            }

            for statement in &function.body.statements {
                validate_statement_types(statement, &structs)?;
            }
        }

        Ok(Self {
            structs,
            functions,
            output: String::new(),
        })
    }

    fn call_main(&mut self) -> Result<Option<Value>, RuntimeError> {
        if !self.functions.contains_key("main") {
            return Err(RuntimeError {
                message: "function `main` not found".to_string(),
            });
        }

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
            let value = self.evaluate_as(argument, parameter.ty.clone(), caller_variables)?;
            variables.insert(parameter.name.clone(), value);
        }

        let returned = self.run_function(function, &mut variables)?;

        match (function.return_type.clone(), returned) {
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
            if let Some(value) =
                self.run_statement(statement, function.return_type.clone(), variables)?
            {
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
                let value = self.evaluate_as(&statement.value, statement.ty.clone(), variables)?;
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
            Expression::StructLiteral(literal) => self.evaluate_struct_literal(literal, variables),
            Expression::FieldAccess(access) => self.evaluate_field_access(access, variables),
            Expression::AddressOf(expression) => {
                let value = self.evaluate(expression, variables)?;
                Ok(Value::Pointer(Box::new(value)))
            }
            Expression::Dereference(expression) => self.evaluate_dereference(expression, variables),
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
            Expression::StructLiteral(literal) => {
                let value = self.evaluate_struct_literal(literal, variables)?;
                expect_type(value, ty)
            }
            Expression::FieldAccess(access) => {
                let value = self.evaluate_field_access(access, variables)?;
                expect_type(value, ty)
            }
            Expression::AddressOf(expression) => {
                let value = self.evaluate(expression, variables)?;
                expect_type(Value::Pointer(Box::new(value)), ty)
            }
            Expression::Dereference(expression) => {
                let value = self.evaluate_dereference(expression, variables)?;
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

    fn evaluate_struct_literal(
        &mut self,
        literal: &StructLiteral,
        variables: &HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let definition = self
            .structs
            .get(&literal.name)
            .copied()
            .ok_or_else(|| RuntimeError {
                message: format!("unknown struct `{}`", literal.name),
            })?;
        let field_types = definition
            .fields
            .iter()
            .map(|field| (field.name.clone(), field.ty.clone()))
            .collect::<Vec<_>>();
        let mut values = HashMap::new();

        for field in &literal.fields {
            if values.contains_key(&field.name) {
                return Err(RuntimeError {
                    message: format!(
                        "field `{}` is already initialized in `{}`",
                        field.name, literal.name
                    ),
                });
            }

            let Some((_, ty)) = field_types.iter().find(|(name, _)| name == &field.name) else {
                return Err(RuntimeError {
                    message: format!("unknown field `{}` in `{}`", field.name, literal.name),
                });
            };
            let value = self.evaluate_as(&field.value, ty.clone(), variables)?;
            values.insert(field.name.clone(), value);
        }

        let mut fields = Vec::new();

        for (field_name, _) in field_types {
            let Some(value) = values.remove(&field_name) else {
                return Err(RuntimeError {
                    message: format!("missing field `{field_name}` in `{}`", literal.name),
                });
            };
            fields.push((field_name, value));
        }

        Ok(Value::Struct {
            name: literal.name.clone(),
            fields,
        })
    }

    fn evaluate_field_access(
        &mut self,
        access: &FieldAccess,
        variables: &HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let object = self.evaluate(&access.object, variables)?;

        match object {
            Value::Struct { name, fields } => fields
                .into_iter()
                .find(|(field, _)| field == &access.field)
                .map(|(_, value)| value)
                .ok_or_else(|| RuntimeError {
                    message: format!("unknown field `{}` in `{name}`", access.field),
                }),
            Value::Pointer(value) => match *value {
                Value::Struct { name, fields } => fields
                    .into_iter()
                    .find(|(field, _)| field == &access.field)
                    .map(|(_, value)| value)
                    .ok_or_else(|| RuntimeError {
                        message: format!("unknown field `{}` in `{name}`", access.field),
                    }),
                value => Err(RuntimeError {
                    message: format!(
                        "cannot access field `{}` on `{}`",
                        access.field,
                        value.ty().name()
                    ),
                }),
            },
            value => Err(RuntimeError {
                message: format!(
                    "cannot access field `{}` on `{}`",
                    access.field,
                    value.ty().name()
                ),
            }),
        }
    }

    fn evaluate_dereference(
        &mut self,
        expression: &Expression,
        variables: &HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let value = self.evaluate(expression, variables)?;

        match value {
            Value::Pointer(value) => Ok(*value),
            value => Err(RuntimeError {
                message: format!("cannot dereference `{}`", value.ty().name()),
            }),
        }
    }
}

fn validate_statement_types(
    statement: &Statement,
    structs: &HashMap<String, &StructDefinition>,
) -> Result<(), RuntimeError> {
    match statement {
        Let(statement) => validate_type(&statement.ty, structs),
        Call(_) | Return(_) => Ok(()),
    }
}

fn validate_type(
    ty: &Type,
    structs: &HashMap<String, &StructDefinition>,
) -> Result<(), RuntimeError> {
    match ty {
        Type::Struct(name) if !structs.contains_key(name) => Err(RuntimeError {
            message: format!("unknown type `{name}`"),
        }),
        Type::Pointer(ty) => validate_type(ty, structs),
        _ => Ok(()),
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
    fn reads_struct_field() {
        let program = parse_source(
            "struct Point { x: i32, y: bool } fn main() { let p: Point = Point { x: 7, y: true } println(p.x) println(p.y) }",
        )
        .unwrap();

        assert_eq!(run_main(&program).unwrap(), "7\ntrue\n");
    }

    #[test]
    fn passes_struct_to_function() {
        let program = parse_source(
            "struct Point { x: i32 } fn pick_x(p: Point) -> i32 { return p.x } fn main() { let p: Point = Point { x: 9 } println(pick_x(p)) }",
        )
        .unwrap();

        assert_eq!(run_main(&program).unwrap(), "9\n");
    }

    #[test]
    fn dereferences_pointer() {
        let program =
            parse_source("fn main() { let a: i32 = 7 let p: &i32 = &a println(*p) }").unwrap();

        assert_eq!(run_main(&program).unwrap(), "7\n");
    }

    #[test]
    fn passes_pointer_to_function() {
        let program =
            parse_source("fn show(p: &i32) { println(*p) } fn main() { let a: i32 = 8 show(&a) }")
                .unwrap();

        assert_eq!(run_main(&program).unwrap(), "8\n");
    }

    #[test]
    fn reads_field_through_struct_pointer() {
        let program = parse_source(
            "struct Point { x: i32 } fn pick_x(p: &Point) -> i32 { return p.x } fn main() { let p: Point = Point { x: 9 } println(pick_x(&p)) }",
        )
        .unwrap();

        assert_eq!(run_main(&program).unwrap(), "9\n");
    }

    #[test]
    fn rejects_dereferencing_non_pointer() {
        let program = parse_source("fn main() { let a: i32 = 7 println(*a) }").unwrap();
        let error = run_main(&program).unwrap_err();

        assert!(error.message.contains("cannot dereference `i32`"));
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
