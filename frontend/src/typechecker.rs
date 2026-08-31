use std::collections::HashMap;

use crate::{
    BinaryExpression, BinaryOperator, Block, CallExpression, Expression, Function, Program,
    Statement, StructField, StructLiteral, Type,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeError {
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedProgram {
    pub structs: HashMap<String, CheckedStruct>,
    pub functions: HashMap<String, FunctionSignature>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedStruct {
    pub fields: Vec<StructField>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionSignature {
    pub parameters: Vec<Type>,
    pub return_type: Option<Type>,
}

pub fn check_program(program: &Program) -> Result<CheckedProgram, TypeError> {
    TypeChecker::new(program)?.check()
}

struct TypeChecker<'program> {
    program: &'program Program,
    structs: HashMap<String, CheckedStruct>,
    functions: HashMap<String, FunctionSignature>,
}

impl<'program> TypeChecker<'program> {
    fn new(program: &'program Program) -> Result<Self, TypeError> {
        let mut structs = HashMap::new();

        for structure in &program.structs {
            if Type::from_name(&structure.name).is_some() {
                return Err(type_error(format!(
                    "cannot define primitive type `{}`",
                    structure.name
                )));
            }

            if structs
                .insert(
                    structure.name.clone(),
                    CheckedStruct {
                        fields: structure.fields.clone(),
                    },
                )
                .is_some()
            {
                return Err(type_error(format!(
                    "struct `{}` is already defined",
                    structure.name
                )));
            }

            let mut fields = HashMap::new();
            for field in &structure.fields {
                if fields.insert(field.name.clone(), ()).is_some() {
                    return Err(type_error(format!(
                        "field `{}` is already defined in struct `{}`",
                        field.name, structure.name
                    )));
                }
            }
        }

        let mut functions = HashMap::new();

        for function in &program.functions {
            if is_builtin_function(&function.name) {
                return Err(type_error(format!(
                    "cannot define builtin function `{}`",
                    function.name
                )));
            }

            let signature = FunctionSignature {
                parameters: function
                    .parameters
                    .iter()
                    .map(|parameter| parameter.ty.clone())
                    .collect(),
                return_type: function.return_type.clone(),
            };

            if functions.insert(function.name.clone(), signature).is_some() {
                return Err(type_error(format!(
                    "function `{}` is already defined",
                    function.name
                )));
            }
        }

        Ok(Self {
            program,
            structs,
            functions,
        })
    }

    fn check(self) -> Result<CheckedProgram, TypeError> {
        for structure in &self.program.structs {
            for field in &structure.fields {
                self.check_declared_type(&field.ty)?;
            }
        }

        for function in &self.program.functions {
            self.check_function(function)?;
        }

        Ok(CheckedProgram {
            structs: self.structs,
            functions: self.functions,
        })
    }

    fn check_function(&self, function: &Function) -> Result<(), TypeError> {
        let mut variables = Vec::new();
        let mut scope = HashMap::new();

        for parameter in &function.parameters {
            self.check_declared_type(&parameter.ty)?;

            if scope
                .insert(parameter.name.clone(), parameter.ty.clone())
                .is_some()
            {
                return Err(type_error(format!(
                    "parameter `{}` is already defined",
                    parameter.name
                )));
            }
        }

        variables.push(scope);

        if let Some(return_type) = &function.return_type {
            self.check_declared_type(return_type)?;
        }

        self.check_block(
            &function.body,
            function.return_type.as_ref(),
            &mut variables,
            0,
        )?;

        if function.return_type.is_some() && !block_guarantees_return(&function.body) {
            return Err(type_error(format!(
                "function `{}` must return `{}`",
                function.name,
                function.return_type.as_ref().unwrap().name()
            )));
        }

        Ok(())
    }

    fn check_block(
        &self,
        block: &Block,
        return_type: Option<&Type>,
        variables: &mut Vec<HashMap<String, Type>>,
        loop_depth: usize,
    ) -> Result<(), TypeError> {
        variables.push(HashMap::new());

        for statement in &block.statements {
            self.check_statement(statement, return_type, variables, loop_depth)?;
        }

        variables.pop();
        Ok(())
    }

    fn check_statement(
        &self,
        statement: &Statement,
        return_type: Option<&Type>,
        variables: &mut Vec<HashMap<String, Type>>,
        loop_depth: usize,
    ) -> Result<(), TypeError> {
        match statement {
            Statement::Let(statement) => {
                self.check_declared_type(&statement.ty)?;
                self.check_expression_as(&statement.value, &statement.ty, variables)?;
                let scope = variables
                    .last_mut()
                    .expect("there is always a current scope");

                if scope
                    .insert(statement.name.clone(), statement.ty.clone())
                    .is_some()
                {
                    return Err(type_error(format!(
                        "variable `{}` is already defined in this scope",
                        statement.name
                    )));
                }

                Ok(())
            }
            Statement::Call(statement) => {
                let call = CallExpression {
                    name: statement.name.clone(),
                    arguments: statement.arguments.clone(),
                };
                self.check_call_statement(&call, variables)
            }
            Statement::Return(statement) => {
                let Some(return_type) = return_type else {
                    return Err(type_error(
                        "cannot return a value from function without return type",
                    ));
                };

                self.check_expression_as(&statement.value, return_type, variables)?;
                Ok(())
            }
            Statement::If(statement) => {
                self.check_expression_as(&statement.condition, &Type::Bool, variables)?;
                self.check_block(&statement.then_block, return_type, variables, loop_depth)?;

                if let Some(block) = &statement.else_block {
                    self.check_block(block, return_type, variables, loop_depth)?;
                }

                Ok(())
            }
            Statement::Loop(statement) => {
                self.check_block(&statement.body, return_type, variables, loop_depth + 1)
            }
            Statement::While(statement) => {
                self.check_expression_as(&statement.condition, &Type::Bool, variables)?;
                self.check_block(&statement.body, return_type, variables, loop_depth + 1)
            }
            Statement::Break if loop_depth == 0 => Err(type_error("cannot `break` outside loop")),
            Statement::Continue if loop_depth == 0 => {
                Err(type_error("cannot `continue` outside loop"))
            }
            Statement::Break | Statement::Continue => Ok(()),
        }
    }

    fn check_call_statement(
        &self,
        call: &CallExpression,
        variables: &mut Vec<HashMap<String, Type>>,
    ) -> Result<(), TypeError> {
        match call.name.as_str() {
            "println" => {
                if call.arguments.len() != 1 {
                    return Err(type_error("function `println` expects 1 argument"));
                }

                self.check_expression(&call.arguments[0], variables, None)?;
                Ok(())
            }
            "dealloc" => {
                if call.arguments.len() != 1 {
                    return Err(type_error("function `dealloc` expects 1 argument"));
                }

                let ty = self.check_expression(&call.arguments[0], variables, None)?;
                if matches!(ty, Type::Pointer(_)) {
                    Ok(())
                } else {
                    Err(type_error(format!(
                        "function `dealloc` expects pointer, found `{}`",
                        ty.name()
                    )))
                }
            }
            "alloc" => {
                self.check_call_expression(call, variables, None)?;
                Ok(())
            }
            _ => {
                self.check_user_call(call, variables)?;
                Ok(())
            }
        }
    }

    fn check_expression_as(
        &self,
        expression: &Expression,
        expected: &Type,
        variables: &mut Vec<HashMap<String, Type>>,
    ) -> Result<Type, TypeError> {
        let actual = self.check_expression(expression, variables, Some(expected))?;
        expect_type(&actual, expected)
    }

    fn check_expression(
        &self,
        expression: &Expression,
        variables: &mut Vec<HashMap<String, Type>>,
        expected: Option<&Type>,
    ) -> Result<Type, TypeError> {
        match expression {
            Expression::Integer(value) => {
                let ty = expected.cloned().unwrap_or(Type::I32);
                self.check_integer_literal(*value, &ty)?;
                Ok(ty)
            }
            Expression::Float(_) => {
                let ty = expected.cloned().unwrap_or(Type::F64);
                if ty.is_float() {
                    Ok(ty)
                } else {
                    Err(type_error(format!(
                        "cannot assign float literal to `{}`",
                        ty.name()
                    )))
                }
            }
            Expression::Bool(_) => {
                let ty = expected.cloned().unwrap_or(Type::Bool);
                expect_type(&Type::Bool, &ty)
            }
            Expression::Variable(name) => {
                let ty = lookup_variable(name, variables)?;
                if let Some(expected) = expected {
                    expect_type(&ty, expected)
                } else {
                    Ok(ty)
                }
            }
            Expression::Call(call) => self.check_call_expression(call, variables, expected),
            Expression::StructLiteral(literal) => {
                self.check_struct_literal(literal, variables, expected)
            }
            Expression::FieldAccess(access) => {
                let object_ty = self.check_expression(&access.object, variables, None)?;
                let struct_name = match object_ty {
                    Type::Struct(name) => name,
                    Type::Pointer(inner) => match *inner {
                        Type::Struct(name) => name,
                        ty => {
                            return Err(type_error(format!(
                                "cannot access field `{}` on `&{}`",
                                access.field,
                                ty.name()
                            )));
                        }
                    },
                    ty => {
                        return Err(type_error(format!(
                            "cannot access field `{}` on `{}`",
                            access.field,
                            ty.name()
                        )));
                    }
                };
                let ty = self.struct_field_type(&struct_name, &access.field)?;

                if let Some(expected) = expected {
                    expect_type(&ty, expected)
                } else {
                    Ok(ty)
                }
            }
            Expression::AddressOf(expression) => {
                if let Some(Type::Pointer(expected)) = expected {
                    self.check_expression_as(expression, expected, variables)?;
                    Ok(Type::Pointer(expected.clone()))
                } else {
                    let ty = self.check_expression(expression, variables, None)?;
                    Ok(Type::Pointer(Box::new(ty)))
                }
            }
            Expression::Dereference(expression) => {
                let ty = self.check_expression(expression, variables, None)?;
                let Type::Pointer(inner) = ty else {
                    return Err(type_error(format!("cannot dereference `{}`", ty.name())));
                };

                if let Some(expected) = expected {
                    expect_type(&inner, expected)
                } else {
                    Ok(*inner)
                }
            }
            Expression::BitNot(expression) => {
                let operand_ty = if let Some(expected) = expected {
                    if expected.is_bool() || expected.is_integer() {
                        self.check_expression_as(expression, expected, variables)?
                    } else {
                        return Err(type_error(format!(
                            "operator `!` cannot be used with `{}`",
                            expected.name()
                        )));
                    }
                } else {
                    self.check_expression(expression, variables, None)?
                };

                if operand_ty.is_bool() || operand_ty.is_integer() {
                    Ok(operand_ty)
                } else {
                    Err(type_error(format!(
                        "operator `!` cannot be used with `{}`",
                        operand_ty.name()
                    )))
                }
            }
            Expression::Binary(binary) => self.check_binary_expression(binary, variables, expected),
        }
    }

    fn check_binary_expression(
        &self,
        binary: &BinaryExpression,
        variables: &mut Vec<HashMap<String, Type>>,
        expected: Option<&Type>,
    ) -> Result<Type, TypeError> {
        match binary.operator {
            BinaryOperator::BoolAnd | BinaryOperator::BoolOr => {
                self.check_expression_as(&binary.left, &Type::Bool, variables)?;
                self.check_expression_as(&binary.right, &Type::Bool, variables)?;
                expect_optional_type(&Type::Bool, expected)
            }
            BinaryOperator::Equal | BinaryOperator::NotEqual => {
                let target = self.infer_binary_operand_type(binary, variables, None)?;
                self.check_expression_as(&binary.left, &target, variables)?;
                self.check_expression_as(&binary.right, &target, variables)?;
                expect_optional_type(&Type::Bool, expected)
            }
            BinaryOperator::Add
            | BinaryOperator::Subtract
            | BinaryOperator::Multiply
            | BinaryOperator::Divide => {
                let target = self.infer_binary_operand_type(binary, variables, expected)?;

                if target.is_integer() || target.is_float() {
                    self.check_expression_as(&binary.left, &target, variables)?;
                    self.check_expression_as(&binary.right, &target, variables)?;
                    Ok(target)
                } else {
                    Err(type_error(format!(
                        "operator `{}` cannot be used with `{}`",
                        binary.operator.symbol(),
                        target.name()
                    )))
                }
            }
            BinaryOperator::BitAnd
            | BinaryOperator::BitOr
            | BinaryOperator::BitXor
            | BinaryOperator::ShiftLeft
            | BinaryOperator::ShiftRight => {
                let target = self.infer_binary_operand_type(binary, variables, expected)?;

                if target.is_integer() {
                    self.check_expression_as(&binary.left, &target, variables)?;
                    self.check_expression_as(&binary.right, &target, variables)?;
                    Ok(target)
                } else {
                    Err(type_error(format!(
                        "operator `{}` cannot be used with `{}`",
                        binary.operator.symbol(),
                        target.name()
                    )))
                }
            }
        }
    }

    fn infer_binary_operand_type(
        &self,
        binary: &BinaryExpression,
        variables: &mut Vec<HashMap<String, Type>>,
        expected: Option<&Type>,
    ) -> Result<Type, TypeError> {
        if let Some(expected) = expected {
            if !binary.operator.result_is_bool() {
                return Ok(expected.clone());
            }
        }

        if is_untyped_literal(&binary.left) && !is_untyped_literal(&binary.right) {
            self.check_expression(&binary.right, variables, None)
        } else {
            self.check_expression(&binary.left, variables, None)
        }
    }

    fn check_call_expression(
        &self,
        call: &CallExpression,
        variables: &mut Vec<HashMap<String, Type>>,
        expected: Option<&Type>,
    ) -> Result<Type, TypeError> {
        match call.name.as_str() {
            "println" => Err(type_error("function `println` does not return a value")),
            "dealloc" => Err(type_error("function `dealloc` does not return a value")),
            "alloc" => self.check_alloc_call(call, variables, expected),
            name => {
                let signature = self.check_user_call(call, variables)?;

                let Some(return_type) = &signature.return_type else {
                    return Err(type_error(format!(
                        "function `{name}` does not return a value"
                    )));
                };

                if let Some(expected) = expected {
                    expect_type(return_type, expected)
                } else {
                    Ok(return_type.clone())
                }
            }
        }
    }

    fn check_user_call(
        &self,
        call: &CallExpression,
        variables: &mut Vec<HashMap<String, Type>>,
    ) -> Result<&FunctionSignature, TypeError> {
        let signature = self
            .functions
            .get(&call.name)
            .ok_or_else(|| type_error(format!("unknown function `{}`", call.name)))?;

        if signature.parameters.len() != call.arguments.len() {
            return Err(type_error(format!(
                "function `{}` expects {} argument(s)",
                call.name,
                signature.parameters.len()
            )));
        }

        for (argument, parameter_ty) in call.arguments.iter().zip(&signature.parameters) {
            self.check_expression_as(argument, parameter_ty, variables)?;
        }

        Ok(signature)
    }

    fn check_alloc_call(
        &self,
        call: &CallExpression,
        variables: &mut Vec<HashMap<String, Type>>,
        expected: Option<&Type>,
    ) -> Result<Type, TypeError> {
        if call.arguments.len() != 1 {
            return Err(type_error("function `alloc` expects 1 argument"));
        }

        if let Some(Type::Pointer(target)) = expected {
            self.check_expression_as(&call.arguments[0], target, variables)?;
            Ok(Type::Pointer(target.clone()))
        } else if expected.is_some() {
            Err(type_error("function `alloc` returns a pointer"))
        } else {
            let value_ty = self.check_expression(&call.arguments[0], variables, None)?;
            Ok(Type::Pointer(Box::new(value_ty)))
        }
    }

    fn check_struct_literal(
        &self,
        literal: &StructLiteral,
        variables: &mut Vec<HashMap<String, Type>>,
        expected: Option<&Type>,
    ) -> Result<Type, TypeError> {
        let ty = if let Some(Type::Struct(name)) = expected {
            if name != &literal.name {
                return Err(type_error(format!(
                    "cannot use `{}` value as `{}`",
                    literal.name, name
                )));
            }
            Type::Struct(name.clone())
        } else if expected.is_some() {
            return Err(type_error(format!(
                "cannot use `{}` value as `{}`",
                literal.name,
                expected.unwrap().name()
            )));
        } else {
            Type::Struct(literal.name.clone())
        };

        let definition = self
            .structs
            .get(&literal.name)
            .ok_or_else(|| type_error(format!("unknown struct `{}`", literal.name)))?;
        let mut initialized = HashMap::new();

        for field in &literal.fields {
            if initialized.insert(field.name.clone(), ()).is_some() {
                return Err(type_error(format!(
                    "field `{}` is already initialized in `{}`",
                    field.name, literal.name
                )));
            }

            let field_ty = definition
                .fields
                .iter()
                .find(|defined| defined.name == field.name)
                .map(|defined| defined.ty.clone())
                .ok_or_else(|| {
                    type_error(format!(
                        "unknown field `{}` in `{}`",
                        field.name, literal.name
                    ))
                })?;
            self.check_expression_as(&field.value, &field_ty, variables)?;
        }

        for field in &definition.fields {
            if !initialized.contains_key(&field.name) {
                return Err(type_error(format!(
                    "missing field `{}` in `{}`",
                    field.name, literal.name
                )));
            }
        }

        Ok(ty)
    }

    fn check_declared_type(&self, ty: &Type) -> Result<(), TypeError> {
        match ty {
            Type::Struct(name) if !self.structs.contains_key(name) => {
                Err(type_error(format!("unknown type `{name}`")))
            }
            Type::Pointer(inner) => self.check_declared_type(inner),
            _ => Ok(()),
        }
    }

    fn check_integer_literal(&self, value: u128, ty: &Type) -> Result<(), TypeError> {
        if !ty.is_integer() {
            return Err(type_error(format!(
                "cannot assign integer literal to `{}`",
                ty.name()
            )));
        }

        let max = ty.max_integer_value().expect("integer type has max value");

        if value > max {
            return Err(type_error(format!(
                "integer literal `{value}` does not fit in `{}`",
                ty.name()
            )));
        }

        Ok(())
    }

    fn struct_field_type(&self, structure: &str, field: &str) -> Result<Type, TypeError> {
        self.structs
            .get(structure)
            .and_then(|structure| {
                structure
                    .fields
                    .iter()
                    .find(|defined| defined.name == field)
            })
            .map(|field| field.ty.clone())
            .ok_or_else(|| type_error(format!("unknown field `{field}` in `{structure}`")))
    }
}

fn lookup_variable(name: &str, variables: &[HashMap<String, Type>]) -> Result<Type, TypeError> {
    variables
        .iter()
        .rev()
        .find_map(|scope| scope.get(name))
        .cloned()
        .ok_or_else(|| type_error(format!("variable `{name}` not found")))
}

fn expect_optional_type(actual: &Type, expected: Option<&Type>) -> Result<Type, TypeError> {
    if let Some(expected) = expected {
        expect_type(actual, expected)
    } else {
        Ok(actual.clone())
    }
}

fn expect_type(actual: &Type, expected: &Type) -> Result<Type, TypeError> {
    if actual == expected {
        Ok(actual.clone())
    } else {
        Err(type_error(format!(
            "cannot use `{}` value as `{}`",
            actual.name(),
            expected.name()
        )))
    }
}

fn block_guarantees_return(block: &Block) -> bool {
    block.statements.iter().any(statement_guarantees_return)
}

fn statement_guarantees_return(statement: &Statement) -> bool {
    match statement {
        Statement::Return(_) => true,
        Statement::If(statement) => {
            let Some(else_block) = &statement.else_block else {
                return false;
            };

            block_guarantees_return(&statement.then_block) && block_guarantees_return(else_block)
        }
        Statement::Loop(statement) => block_guarantees_return(&statement.body),
        _ => false,
    }
}

fn is_untyped_literal(expression: &Expression) -> bool {
    matches!(expression, Expression::Integer(_) | Expression::Float(_))
}

fn is_builtin_function(name: &str) -> bool {
    matches!(name, "println" | "alloc" | "dealloc")
}

fn type_error(message: impl Into<String>) -> TypeError {
    TypeError {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use crate::{check_program, parse_source};

    #[test]
    fn accepts_typed_program() {
        let program = parse_source(
            "fn add(a: i32, b: i32) -> i32 { return a + b } fn main() { println(add(1, 2)) }",
        )
        .unwrap();

        check_program(&program).unwrap();
    }

    #[test]
    fn accepts_void_function_call_statement() {
        let program =
            parse_source("fn log(value: i32) { println(value) } fn main() { log(7) }").unwrap();

        check_program(&program).unwrap();
    }

    #[test]
    fn rejects_mismatched_math_types() {
        let program =
            parse_source("fn main() { let a: i32 = 1 let b: u32 = 2 println(a + b) }").unwrap();
        let error = check_program(&program).unwrap_err();

        assert!(error.message.contains("cannot use `u32` value as `i32`"));
    }

    #[test]
    fn rejects_non_bool_if_condition() {
        let program = parse_source("fn main() { if 1 { println(1) } }").unwrap();
        let error = check_program(&program).unwrap_err();

        assert!(
            error
                .message
                .contains("cannot assign integer literal to `bool`")
        );
    }

    #[test]
    fn rejects_break_outside_loop() {
        let program = parse_source("fn main() { break }").unwrap();
        let error = check_program(&program).unwrap_err();

        assert!(error.message.contains("cannot `break` outside loop"));
    }

    #[test]
    fn rejects_missing_return() {
        let program = parse_source("fn sample() -> i32 { if true { return 1 } }").unwrap();
        let error = check_program(&program).unwrap_err();

        assert!(error.message.contains("must return `i32`"));
    }

    #[test]
    fn rejects_unknown_type() {
        let program = parse_source("fn main() { let value: Missing = Missing { x: 1 } }").unwrap();
        let error = check_program(&program).unwrap_err();

        assert!(error.message.contains("unknown type `Missing`"));
    }
}
