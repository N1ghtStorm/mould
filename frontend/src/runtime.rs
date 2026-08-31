use std::collections::HashMap;

use crate::{
    BinaryExpression, BinaryOperator, Block, CallExpression, Expression, FieldAccess, Function,
    Program,
    Statement::{self, Break, Call, Continue, If, Let, Loop, Return, While},
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
        value: IntegerValue,
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
    Pointer(PointerValue),
}

#[derive(Debug, Clone, PartialEq)]
enum IntegerValue {
    Signed(i128),
    Unsigned(u128),
}

#[derive(Debug, Clone, PartialEq)]
enum PointerValue {
    Reference(Box<Value>),
    Heap { allocation: usize, ty: Type },
}

#[derive(Debug, Clone, PartialEq)]
enum ExecutionSignal {
    None,
    Return(Value),
    Break,
    Continue,
}

impl Value {
    fn ty(&self) -> Type {
        match self {
            Self::Integer { ty, .. } | Self::Float { ty, .. } => ty.clone(),
            Self::Bool(_) => Type::Bool,
            Self::Struct { name, .. } => Type::Struct(name.clone()),
            Self::Pointer(pointer) => Type::Pointer(Box::new(pointer.ty())),
        }
    }

    fn printable(&self) -> String {
        match self {
            Self::Integer { value, .. } => value.printable(),
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
            Self::Pointer(pointer) => pointer.printable(),
        }
    }
}

impl IntegerValue {
    fn printable(&self) -> String {
        match self {
            Self::Signed(value) => value.to_string(),
            Self::Unsigned(value) => value.to_string(),
        }
    }
}

impl PointerValue {
    fn ty(&self) -> Type {
        match self {
            Self::Reference(value) => value.ty(),
            Self::Heap { ty, .. } => ty.clone(),
        }
    }

    fn printable(&self) -> String {
        match self {
            Self::Reference(value) => format!("&{}", value.printable()),
            Self::Heap { allocation, ty } => format!("&{}#{allocation}", ty.name()),
        }
    }
}

struct Runtime<'program> {
    structs: HashMap<String, &'program StructDefinition>,
    functions: HashMap<String, &'program Function>,
    heap: HashMap<usize, Value>,
    next_allocation: usize,
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
            if is_builtin_function(&function.name) {
                return Err(RuntimeError {
                    message: format!("cannot define builtin function `{}`", function.name),
                });
            }

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
            heap: HashMap::new(),
            next_allocation: 0,
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
        match self.run_block(&function.body, function.return_type.clone(), variables)? {
            ExecutionSignal::None => Ok(None),
            ExecutionSignal::Return(value) => Ok(Some(value)),
            ExecutionSignal::Break => Err(RuntimeError {
                message: "cannot `break` outside loop".to_string(),
            }),
            ExecutionSignal::Continue => Err(RuntimeError {
                message: "cannot `continue` outside loop".to_string(),
            }),
        }
    }

    fn run_block(
        &mut self,
        block: &Block,
        return_type: Option<Type>,
        variables: &mut HashMap<String, Value>,
    ) -> Result<ExecutionSignal, RuntimeError> {
        for statement in &block.statements {
            let signal = self.run_statement(statement, return_type.clone(), variables)?;
            if signal != ExecutionSignal::None {
                return Ok(signal);
            }
        }

        Ok(ExecutionSignal::None)
    }

    fn run_statement(
        &mut self,
        statement: &Statement,
        return_type: Option<Type>,
        variables: &mut HashMap<String, Value>,
    ) -> Result<ExecutionSignal, RuntimeError> {
        match statement {
            Let(statement) => {
                let value = self.evaluate_as(&statement.value, statement.ty.clone(), variables)?;
                variables.insert(statement.name.clone(), value);
                Ok(ExecutionSignal::None)
            }
            Call(statement) => {
                let call = CallExpression {
                    name: statement.name.clone(),
                    arguments: statement.arguments.clone(),
                };
                self.evaluate_call_statement(&call, variables)?;
                Ok(ExecutionSignal::None)
            }
            Return(statement) => {
                let Some(ty) = return_type else {
                    return Err(RuntimeError {
                        message: "cannot return a value from function without return type"
                            .to_string(),
                    });
                };

                self.evaluate_as(&statement.value, ty, variables)
                    .map(ExecutionSignal::Return)
            }
            If(statement) => {
                let condition = self.evaluate_as(&statement.condition, Type::Bool, variables)?;
                let Value::Bool(condition) = condition else {
                    unreachable!("bool type is stored as bool value");
                };

                let block = if condition {
                    Some(&statement.then_block)
                } else {
                    statement.else_block.as_ref()
                };

                if let Some(block) = block {
                    let mut block_variables = variables.clone();
                    return self.run_block(block, return_type, &mut block_variables);
                }

                Ok(ExecutionSignal::None)
            }
            Loop(statement) => loop {
                let mut block_variables = variables.clone();
                match self.run_block(&statement.body, return_type.clone(), &mut block_variables)? {
                    ExecutionSignal::None | ExecutionSignal::Continue => {}
                    ExecutionSignal::Break => return Ok(ExecutionSignal::None),
                    signal @ ExecutionSignal::Return(_) => return Ok(signal),
                }
            },
            While(statement) => loop {
                let condition = self.evaluate_as(&statement.condition, Type::Bool, variables)?;
                let Value::Bool(condition) = condition else {
                    unreachable!("bool type is stored as bool value");
                };

                if !condition {
                    return Ok(ExecutionSignal::None);
                }

                let mut block_variables = variables.clone();
                match self.run_block(&statement.body, return_type.clone(), &mut block_variables)? {
                    ExecutionSignal::None | ExecutionSignal::Continue => {}
                    ExecutionSignal::Break => return Ok(ExecutionSignal::None),
                    signal @ ExecutionSignal::Return(_) => return Ok(signal),
                }
            },
            Break => Ok(ExecutionSignal::Break),
            Continue => Ok(ExecutionSignal::Continue),
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
                Ok(Value::Pointer(PointerValue::Reference(Box::new(value))))
            }
            Expression::Dereference(expression) => self.evaluate_dereference(expression, variables),
            Expression::BitNot(expression) => self.evaluate_bit_not(expression, variables),
            Expression::Binary(binary) => self.evaluate_binary(binary, variables),
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
                if call.name == "alloc" {
                    return self.evaluate_alloc_as(call, ty, variables);
                }

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
                expect_type(Value::Pointer(PointerValue::Reference(Box::new(value))), ty)
            }
            Expression::Dereference(expression) => {
                let value = self.evaluate_dereference(expression, variables)?;
                expect_type(value, ty)
            }
            Expression::BitNot(expression) => self.evaluate_bit_not_as(expression, ty, variables),
            Expression::Binary(binary) => self.evaluate_binary_as(binary, ty, variables),
        }
    }

    fn evaluate_binary(
        &mut self,
        binary: &BinaryExpression,
        variables: &HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        match binary.operator {
            BinaryOperator::BoolAnd => return self.evaluate_bool_and(binary, variables),
            BinaryOperator::BoolOr => return self.evaluate_bool_or(binary, variables),
            BinaryOperator::Equal | BinaryOperator::NotEqual => {
                return self.evaluate_equality(binary, variables);
            }
            _ => {}
        }

        if is_untyped_numeric_literal(&binary.left) && !is_untyped_numeric_literal(&binary.right) {
            let right = self.evaluate(&binary.right, variables)?;
            let ty = right.ty();
            ensure_binary_operator(binary.operator, &ty)?;
            let left = self.evaluate_as(&binary.left, ty, variables)?;
            return apply_binary_operator(left, binary.operator, right);
        }

        let left = self.evaluate(&binary.left, variables)?;
        let ty = left.ty();
        ensure_binary_operator(binary.operator, &ty)?;
        let right = self.evaluate_as(&binary.right, ty, variables)?;
        apply_binary_operator(left, binary.operator, right)
    }

    fn evaluate_binary_as(
        &mut self,
        binary: &BinaryExpression,
        ty: Type,
        variables: &HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if binary.operator.result_is_bool() {
            let value = self.evaluate_binary(binary, variables)?;
            return expect_type(value, ty);
        }

        ensure_binary_operator(binary.operator, &ty)?;
        let left = self.evaluate_as(&binary.left, ty.clone(), variables)?;
        let right = self.evaluate_as(&binary.right, ty, variables)?;

        apply_binary_operator(left, binary.operator, right)
    }

    fn evaluate_bool_and(
        &mut self,
        binary: &BinaryExpression,
        variables: &HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let left = self.evaluate_as(&binary.left, Type::Bool, variables)?;
        let Value::Bool(left) = left else {
            unreachable!("bool type is stored as bool value");
        };

        if !left {
            return Ok(Value::Bool(false));
        }

        let right = self.evaluate_as(&binary.right, Type::Bool, variables)?;
        let Value::Bool(right) = right else {
            unreachable!("bool type is stored as bool value");
        };

        Ok(Value::Bool(right))
    }

    fn evaluate_bool_or(
        &mut self,
        binary: &BinaryExpression,
        variables: &HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let left = self.evaluate_as(&binary.left, Type::Bool, variables)?;
        let Value::Bool(left) = left else {
            unreachable!("bool type is stored as bool value");
        };

        if left {
            return Ok(Value::Bool(true));
        }

        let right = self.evaluate_as(&binary.right, Type::Bool, variables)?;
        let Value::Bool(right) = right else {
            unreachable!("bool type is stored as bool value");
        };

        Ok(Value::Bool(right))
    }

    fn evaluate_equality(
        &mut self,
        binary: &BinaryExpression,
        variables: &HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let (left, right) = if is_untyped_numeric_literal(&binary.left)
            && !is_untyped_numeric_literal(&binary.right)
        {
            let right = self.evaluate(&binary.right, variables)?;
            let left = self.evaluate_as(&binary.left, right.ty(), variables)?;
            (left, right)
        } else {
            let left = self.evaluate(&binary.left, variables)?;
            let right = self.evaluate_as(&binary.right, left.ty(), variables)?;
            (left, right)
        };

        let equals = left == right;
        Ok(Value::Bool(if binary.operator == BinaryOperator::Equal {
            equals
        } else {
            !equals
        }))
    }

    fn evaluate_bit_not(
        &mut self,
        expression: &Expression,
        variables: &HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let value = self.evaluate(expression, variables)?;
        apply_bit_not_operator(value)
    }

    fn evaluate_bit_not_as(
        &mut self,
        expression: &Expression,
        ty: Type,
        variables: &HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if !ty.is_bool() {
            ensure_integer_operator("!", &ty)?;
        }

        let value = self.evaluate_as(expression, ty, variables)?;
        apply_bit_not_operator(value)
    }

    fn evaluate_call_statement(
        &mut self,
        call: &CallExpression,
        variables: &HashMap<String, Value>,
    ) -> Result<(), RuntimeError> {
        if call.name == "println" {
            return self.evaluate_println(call, variables);
        }

        if call.name == "dealloc" {
            return self.evaluate_dealloc(call, variables);
        }

        if call.name == "alloc" {
            self.evaluate_alloc(call, variables)?;
            return Ok(());
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

        if call.name == "dealloc" {
            return Err(RuntimeError {
                message: "function `dealloc` does not return a value".to_string(),
            });
        }

        if call.name == "alloc" {
            return self.evaluate_alloc(call, variables);
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

    fn evaluate_alloc(
        &mut self,
        call: &CallExpression,
        variables: &HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        if call.arguments.len() != 1 {
            return Err(RuntimeError {
                message: "function `alloc` expects 1 argument".to_string(),
            });
        }

        let value = self.evaluate(&call.arguments[0], variables)?;
        Ok(self.allocate(value))
    }

    fn evaluate_alloc_as(
        &mut self,
        call: &CallExpression,
        ty: Type,
        variables: &HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let Type::Pointer(target_ty) = ty else {
            return Err(RuntimeError {
                message: "function `alloc` returns a pointer".to_string(),
            });
        };

        if call.arguments.len() != 1 {
            return Err(RuntimeError {
                message: "function `alloc` expects 1 argument".to_string(),
            });
        }

        let value = self.evaluate_as(&call.arguments[0], *target_ty, variables)?;
        Ok(self.allocate(value))
    }

    fn evaluate_dealloc(
        &mut self,
        call: &CallExpression,
        variables: &HashMap<String, Value>,
    ) -> Result<(), RuntimeError> {
        if call.arguments.len() != 1 {
            return Err(RuntimeError {
                message: "function `dealloc` expects 1 argument".to_string(),
            });
        }

        let value = self.evaluate(&call.arguments[0], variables)?;

        match value {
            Value::Pointer(PointerValue::Heap { allocation, .. }) => {
                if self.heap.remove(&allocation).is_some() {
                    Ok(())
                } else {
                    Err(RuntimeError {
                        message: "allocation is already freed".to_string(),
                    })
                }
            }
            Value::Pointer(PointerValue::Reference(_)) => Err(RuntimeError {
                message: "function `dealloc` expects pointer returned by `alloc`".to_string(),
            }),
            value => Err(RuntimeError {
                message: format!(
                    "function `dealloc` expects pointer, found `{}`",
                    value.ty().name()
                ),
            }),
        }
    }

    fn allocate(&mut self, value: Value) -> Value {
        let allocation = self.next_allocation;
        let ty = value.ty();
        self.next_allocation += 1;
        self.heap.insert(allocation, value);

        Value::Pointer(PointerValue::Heap { allocation, ty })
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
            Value::Pointer(pointer) => {
                let value = self.dereference_pointer(pointer)?;
                self.get_field(value, &access.field)
            }
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
            Value::Pointer(pointer) => self.dereference_pointer(pointer),
            value => Err(RuntimeError {
                message: format!("cannot dereference `{}`", value.ty().name()),
            }),
        }
    }

    fn dereference_pointer(&self, pointer: PointerValue) -> Result<Value, RuntimeError> {
        match pointer {
            PointerValue::Reference(value) => Ok(*value),
            PointerValue::Heap { allocation, .. } => self
                .heap
                .get(&allocation)
                .cloned()
                .ok_or_else(|| RuntimeError {
                    message: "cannot dereference freed pointer".to_string(),
                }),
        }
    }

    fn get_field(&self, value: Value, field: &str) -> Result<Value, RuntimeError> {
        match value {
            Value::Struct { name, fields } => fields
                .into_iter()
                .find(|(field_name, _)| field_name == field)
                .map(|(_, value)| value)
                .ok_or_else(|| RuntimeError {
                    message: format!("unknown field `{field}` in `{name}`"),
                }),
            value => Err(RuntimeError {
                message: format!("cannot access field `{field}` on `{}`", value.ty().name()),
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
        If(statement) => {
            for statement in &statement.then_block.statements {
                validate_statement_types(statement, structs)?;
            }

            if let Some(block) = &statement.else_block {
                for statement in &block.statements {
                    validate_statement_types(statement, structs)?;
                }
            }

            Ok(())
        }
        Loop(statement) => {
            for statement in &statement.body.statements {
                validate_statement_types(statement, structs)?;
            }

            Ok(())
        }
        While(statement) => {
            for statement in &statement.body.statements {
                validate_statement_types(statement, structs)?;
            }

            Ok(())
        }
        Call(_) | Return(_) | Break | Continue => Ok(()),
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

fn is_builtin_function(name: &str) -> bool {
    matches!(name, "println" | "alloc" | "dealloc")
}

fn apply_binary_operator(
    left: Value,
    operator: BinaryOperator,
    right: Value,
) -> Result<Value, RuntimeError> {
    let ty = left.ty();

    if right.ty() != ty {
        return Err(RuntimeError {
            message: format!(
                "operator `{}` expects matching types, found `{}` and `{}`",
                operator.symbol(),
                ty.name(),
                right.ty().name()
            ),
        });
    }

    if ty.is_integer() {
        let Value::Integer { value: left, .. } = left else {
            unreachable!("integer type is stored as integer value");
        };
        let Value::Integer { value: right, .. } = right else {
            unreachable!("integer type is stored as integer value");
        };

        return apply_integer_operator(left, operator, right, ty);
    }

    if ty.is_float() {
        let Value::Float { value: left, .. } = left else {
            unreachable!("float type is stored as float value");
        };
        let Value::Float { value: right, .. } = right else {
            unreachable!("float type is stored as float value");
        };

        return apply_float_operator(left, operator, right, ty);
    }

    Err(RuntimeError {
        message: format!(
            "operator `{}` cannot be used with `{}`",
            operator.symbol(),
            ty.name()
        ),
    })
}

fn apply_integer_operator(
    left: IntegerValue,
    operator: BinaryOperator,
    right: IntegerValue,
    ty: Type,
) -> Result<Value, RuntimeError> {
    match operator {
        BinaryOperator::Add
        | BinaryOperator::Subtract
        | BinaryOperator::Multiply
        | BinaryOperator::Divide => apply_integer_arithmetic_operator(left, operator, right, ty),
        BinaryOperator::BitAnd
        | BinaryOperator::BitOr
        | BinaryOperator::BitXor
        | BinaryOperator::ShiftLeft
        | BinaryOperator::ShiftRight => apply_integer_bitwise_operator(left, operator, right, ty),
        _ => unreachable!("checked by caller"),
    }
}

fn apply_integer_arithmetic_operator(
    left: IntegerValue,
    operator: BinaryOperator,
    right: IntegerValue,
    ty: Type,
) -> Result<Value, RuntimeError> {
    if ty.is_signed_integer() {
        return apply_signed_integer_arithmetic_operator(left, operator, right, ty);
    }

    let left = unsigned_integer(left);
    let right = unsigned_integer(right);

    if operator == BinaryOperator::Divide && right == 0 {
        return Err(RuntimeError {
            message: "division by zero".to_string(),
        });
    }

    let result = match operator {
        BinaryOperator::Add => left.checked_add(right),
        BinaryOperator::Subtract => left.checked_sub(right),
        BinaryOperator::Multiply => left.checked_mul(right),
        BinaryOperator::Divide => left.checked_div(right),
        _ => unreachable!("checked by caller"),
    }
    .ok_or_else(|| integer_operation_overflow(&ty))?;

    let max = ty
        .max_integer_value()
        .expect("unsigned integer type has max");

    if result > max {
        return Err(integer_operation_overflow(&ty));
    }

    Ok(Value::Integer {
        value: IntegerValue::Unsigned(result),
        ty,
    })
}

fn apply_signed_integer_arithmetic_operator(
    left: IntegerValue,
    operator: BinaryOperator,
    right: IntegerValue,
    ty: Type,
) -> Result<Value, RuntimeError> {
    let left = signed_integer(left);
    let right = signed_integer(right);

    if operator == BinaryOperator::Divide && right == 0 {
        return Err(RuntimeError {
            message: "division by zero".to_string(),
        });
    }

    let result = match operator {
        BinaryOperator::Add => left.checked_add(right),
        BinaryOperator::Subtract => left.checked_sub(right),
        BinaryOperator::Multiply => left.checked_mul(right),
        BinaryOperator::Divide => left.checked_div(right),
        _ => unreachable!("checked by caller"),
    }
    .ok_or_else(|| integer_operation_overflow(&ty))?;

    let (min, max) = signed_integer_bounds(&ty).expect("signed integer type has bounds");

    if result < min || result > max {
        return Err(integer_operation_overflow(&ty));
    }

    Ok(Value::Integer {
        value: IntegerValue::Signed(result),
        ty,
    })
}

fn apply_integer_bitwise_operator(
    left: IntegerValue,
    operator: BinaryOperator,
    right: IntegerValue,
    ty: Type,
) -> Result<Value, RuntimeError> {
    let bits = integer_bits(&ty).expect("integer type has bit width");

    if ty.is_signed_integer() {
        let left = signed_integer(left);
        let right = signed_integer(right);
        let value = match operator {
            BinaryOperator::BitAnd => bits_to_signed(
                signed_to_bits(left, bits) & signed_to_bits(right, bits),
                bits,
            ),
            BinaryOperator::BitOr => bits_to_signed(
                signed_to_bits(left, bits) | signed_to_bits(right, bits),
                bits,
            ),
            BinaryOperator::BitXor => bits_to_signed(
                signed_to_bits(left, bits) ^ signed_to_bits(right, bits),
                bits,
            ),
            BinaryOperator::ShiftLeft => {
                let shift = signed_shift_amount(right, bits, &ty)?;
                bits_to_signed(
                    (signed_to_bits(left, bits) << shift) & integer_mask(bits),
                    bits,
                )
            }
            BinaryOperator::ShiftRight => {
                let shift = signed_shift_amount(right, bits, &ty)?;
                left >> shift
            }
            _ => unreachable!("checked by caller"),
        };

        return Ok(Value::Integer {
            value: IntegerValue::Signed(value),
            ty,
        });
    }

    let left = unsigned_integer(left);
    let right = unsigned_integer(right);
    let value = match operator {
        BinaryOperator::BitAnd => left & right,
        BinaryOperator::BitOr => left | right,
        BinaryOperator::BitXor => left ^ right,
        BinaryOperator::ShiftLeft => {
            let shift = unsigned_shift_amount(right, bits, &ty)?;
            (left << shift) & integer_mask(bits)
        }
        BinaryOperator::ShiftRight => {
            let shift = unsigned_shift_amount(right, bits, &ty)?;
            left >> shift
        }
        _ => unreachable!("checked by caller"),
    } & integer_mask(bits);

    Ok(Value::Integer {
        value: IntegerValue::Unsigned(value),
        ty,
    })
}

fn apply_float_operator(
    left: f64,
    operator: BinaryOperator,
    right: f64,
    ty: Type,
) -> Result<Value, RuntimeError> {
    if operator == BinaryOperator::Divide && right == 0.0 {
        return Err(RuntimeError {
            message: "division by zero".to_string(),
        });
    }

    let value = match operator {
        BinaryOperator::Add => left + right,
        BinaryOperator::Subtract => left - right,
        BinaryOperator::Multiply => left * right,
        BinaryOperator::Divide => left / right,
        _ => {
            return Err(RuntimeError {
                message: format!(
                    "operator `{}` cannot be used with `{}`",
                    operator.symbol(),
                    ty.name()
                ),
            });
        }
    };

    Ok(Value::Float { value, ty })
}

fn apply_bit_not_operator(value: Value) -> Result<Value, RuntimeError> {
    let ty = value.ty();

    if ty.is_bool() {
        let Value::Bool(value) = value else {
            unreachable!("bool type is stored as bool value");
        };

        return Ok(Value::Bool(!value));
    }

    if !ty.is_integer() {
        return Err(RuntimeError {
            message: format!("operator `!` cannot be used with `{}`", ty.name()),
        });
    }

    let Value::Integer { value, ty } = value else {
        unreachable!("integer type is stored as integer value");
    };
    let bits = integer_bits(&ty).expect("integer type has bit width");

    if ty.is_signed_integer() {
        let value = signed_integer(value);
        let value = bits_to_signed(!signed_to_bits(value, bits) & integer_mask(bits), bits);

        return Ok(Value::Integer {
            value: IntegerValue::Signed(value),
            ty,
        });
    }

    let value = unsigned_integer(value);

    Ok(Value::Integer {
        value: IntegerValue::Unsigned(!value & integer_mask(bits)),
        ty,
    })
}

fn ensure_binary_operator(operator: BinaryOperator, ty: &Type) -> Result<(), RuntimeError> {
    if operator.is_bitwise() {
        return ensure_integer_operator(operator.symbol(), ty);
    }

    if ty.is_integer() || ty.is_float() {
        return Ok(());
    }

    Err(RuntimeError {
        message: format!(
            "operator `{}` cannot be used with `{}`",
            operator.symbol(),
            ty.name()
        ),
    })
}

fn ensure_integer_operator(operator: &str, ty: &Type) -> Result<(), RuntimeError> {
    if ty.is_integer() {
        return Ok(());
    }

    Err(RuntimeError {
        message: format!("operator `{operator}` cannot be used with `{}`", ty.name()),
    })
}

fn signed_integer(value: IntegerValue) -> i128 {
    match value {
        IntegerValue::Signed(value) => value,
        IntegerValue::Unsigned(value) => {
            i128::try_from(value).expect("signed integer value fits in i128")
        }
    }
}

fn unsigned_integer(value: IntegerValue) -> u128 {
    match value {
        IntegerValue::Signed(value) => {
            u128::try_from(value).expect("unsigned integer value is non-negative")
        }
        IntegerValue::Unsigned(value) => value,
    }
}

fn signed_to_bits(value: i128, bits: u32) -> u128 {
    (value as u128) & integer_mask(bits)
}

fn bits_to_signed(value: u128, bits: u32) -> i128 {
    let value = value & integer_mask(bits);

    if bits == 128 {
        return value as i128;
    }

    let sign_bit = 1u128 << (bits - 1);

    if value & sign_bit == 0 {
        value as i128
    } else {
        (value as i128) - (1i128 << bits)
    }
}

fn signed_shift_amount(value: i128, bits: u32, ty: &Type) -> Result<u32, RuntimeError> {
    if value < 0 {
        return Err(RuntimeError {
            message: "shift amount cannot be negative".to_string(),
        });
    }

    unsigned_shift_amount(value as u128, bits, ty)
}

fn unsigned_shift_amount(value: u128, bits: u32, ty: &Type) -> Result<u32, RuntimeError> {
    if value >= bits as u128 {
        return Err(RuntimeError {
            message: format!(
                "shift amount must be less than bit width of `{}`",
                ty.name()
            ),
        });
    }

    Ok(value as u32)
}

fn integer_bits(ty: &Type) -> Option<u32> {
    match ty {
        Type::I8 | Type::U8 => Some(8),
        Type::I16 | Type::U16 => Some(16),
        Type::I32 | Type::U32 => Some(32),
        Type::I64 | Type::U64 | Type::Isize | Type::Usize => Some(64),
        Type::I128 | Type::U128 => Some(128),
        _ => None,
    }
}

fn integer_mask(bits: u32) -> u128 {
    if bits == 128 {
        u128::MAX
    } else {
        (1u128 << bits) - 1
    }
}

fn signed_integer_bounds(ty: &Type) -> Option<(i128, i128)> {
    match ty {
        Type::I8 => Some((i8::MIN as i128, i8::MAX as i128)),
        Type::I16 => Some((i16::MIN as i128, i16::MAX as i128)),
        Type::I32 => Some((i32::MIN as i128, i32::MAX as i128)),
        Type::I64 | Type::Isize => Some((i64::MIN as i128, i64::MAX as i128)),
        Type::I128 => Some((i128::MIN, i128::MAX)),
        _ => None,
    }
}

fn integer_operation_overflow(ty: &Type) -> RuntimeError {
    RuntimeError {
        message: format!("operation result does not fit in `{}`", ty.name()),
    }
}

fn is_untyped_numeric_literal(expression: &Expression) -> bool {
    matches!(expression, Expression::Integer(_) | Expression::Float(_))
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

    let value = if ty.is_signed_integer() {
        let value = i128::try_from(value).map_err(|_| RuntimeError {
            message: format!("integer literal `{value}` does not fit in `{}`", ty.name()),
        })?;
        IntegerValue::Signed(value)
    } else {
        IntegerValue::Unsigned(value)
    };

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
    fn evaluates_integer_math_with_precedence() {
        let program = parse_source("fn main() { let n: i32 = 1 + 2 * 3 println(n) }").unwrap();

        assert_eq!(run_main(&program).unwrap(), "7\n");
    }

    #[test]
    fn evaluates_all_integer_math_operators() {
        let program =
            parse_source("fn main() { let n: i32 = 20 + 6 - 3 * 4 / 2 println(n) }").unwrap();

        assert_eq!(run_main(&program).unwrap(), "20\n");
    }

    #[test]
    fn evaluates_signed_negative_result() {
        let program = parse_source("fn main() { let n: i32 = 1 - 2 println(n) }").unwrap();

        assert_eq!(run_main(&program).unwrap(), "-1\n");
    }

    #[test]
    fn infers_literal_from_variable_type_in_math() {
        let program =
            parse_source("fn main() { let a: u8 = 2 let b: u8 = a + 1 println(b) }").unwrap();

        assert_eq!(run_main(&program).unwrap(), "3\n");
    }

    #[test]
    fn evaluates_float_math() {
        let program = parse_source("fn main() { let n: f64 = 1.5 + 2.25 println(n) }").unwrap();

        assert_eq!(run_main(&program).unwrap(), "3.75\n");
    }

    #[test]
    fn rejects_math_with_mismatched_variable_types() {
        let program =
            parse_source("fn main() { let a: i32 = 1 let b: u32 = 2 println(a + b) }").unwrap();
        let error = run_main(&program).unwrap_err();

        assert!(error.message.contains("cannot use `u32` value as `i32`"));
    }

    #[test]
    fn rejects_math_on_bool() {
        let program = parse_source("fn main() { let value: bool = true + false }").unwrap();
        let error = run_main(&program).unwrap_err();

        assert!(
            error
                .message
                .contains("operator `+` cannot be used with `bool`")
        );
    }

    #[test]
    fn rejects_integer_division_by_zero() {
        let program = parse_source("fn main() { let value: i32 = 1 / 0 }").unwrap();
        let error = run_main(&program).unwrap_err();

        assert!(error.message.contains("division by zero"));
    }

    #[test]
    fn rejects_unsigned_underflow() {
        let program = parse_source("fn main() { let value: u8 = 1 - 2 }").unwrap();
        let error = run_main(&program).unwrap_err();

        assert!(error.message.contains("does not fit in `u8`"));
    }

    #[test]
    fn evaluates_bitwise_integer_operators() {
        let program = parse_source(
            "fn main() { let a: u8 = 10 let b: u8 = 12 println(a & b) println(a | b) println(a ^ b) }",
        )
        .unwrap();

        assert_eq!(run_main(&program).unwrap(), "8\n14\n6\n");
    }

    #[test]
    fn evaluates_bit_not_with_type_width() {
        let program = parse_source("fn main() { let value: u8 = !0 println(value) }").unwrap();

        assert_eq!(run_main(&program).unwrap(), "255\n");
    }

    #[test]
    fn evaluates_signed_bit_not() {
        let program = parse_source("fn main() { let value: i32 = !0 println(value) }").unwrap();

        assert_eq!(run_main(&program).unwrap(), "-1\n");
    }

    #[test]
    fn evaluates_shift_operators() {
        let program = parse_source(
            "fn main() { let left: u8 = 1 << 3 let right: u8 = left >> 2 println(left) println(right) }",
        )
        .unwrap();

        assert_eq!(run_main(&program).unwrap(), "8\n2\n");
    }

    #[test]
    fn rejects_bitwise_float() {
        let program = parse_source("fn main() { let value: f64 = 1.5 & 2.5 }").unwrap();
        let error = run_main(&program).unwrap_err();

        assert!(
            error
                .message
                .contains("operator `&` cannot be used with `f64`")
        );
    }

    #[test]
    fn rejects_bit_not_float() {
        let program = parse_source("fn main() { let value: f64 = !1.5 }").unwrap();
        let error = run_main(&program).unwrap_err();

        assert!(
            error
                .message
                .contains("operator `!` cannot be used with `f64`")
        );
    }

    #[test]
    fn rejects_shift_amount_too_large() {
        let program = parse_source("fn main() { let value: u8 = 1 << 8 }").unwrap();
        let error = run_main(&program).unwrap_err();

        assert!(error.message.contains("shift amount must be less"));
    }

    #[test]
    fn evaluates_bool_operators() {
        let program = parse_source(
            "fn main() { println(true && false) println(true || false) println(!true) }",
        )
        .unwrap();

        assert_eq!(run_main(&program).unwrap(), "false\ntrue\nfalse\n");
    }

    #[test]
    fn evaluates_equality_operators() {
        let program = parse_source(
            "fn main() { let a: u8 = 7 println(a == 7) println(a != 8) println(true == false) }",
        )
        .unwrap();

        assert_eq!(run_main(&program).unwrap(), "true\ntrue\nfalse\n");
    }

    #[test]
    fn runs_if_then_branch() {
        let program =
            parse_source("fn main() { if true { println(1) } else { println(2) } }").unwrap();

        assert_eq!(run_main(&program).unwrap(), "1\n");
    }

    #[test]
    fn runs_if_else_branch() {
        let program =
            parse_source("fn main() { if false { println(1) } else { println(2) } }").unwrap();

        assert_eq!(run_main(&program).unwrap(), "2\n");
    }

    #[test]
    fn runs_else_if_branch() {
        let program = parse_source(
            "fn main() { if false { println(1) } else if true { println(2) } else { println(3) } }",
        )
        .unwrap();

        assert_eq!(run_main(&program).unwrap(), "2\n");
    }

    #[test]
    fn returns_from_if_branch() {
        let program = parse_source(
            "fn pick(flag: bool) -> i32 { if flag { return 1 } else { return 2 } } fn main() { println(pick(false)) }",
        )
        .unwrap();

        assert_eq!(run_main(&program).unwrap(), "2\n");
    }

    #[test]
    fn short_circuits_bool_operators() {
        let program =
            parse_source("fn main() { println(false && missing()) println(true || missing()) }")
                .unwrap();

        assert_eq!(run_main(&program).unwrap(), "false\ntrue\n");
    }

    #[test]
    fn rejects_non_bool_if_condition() {
        let program = parse_source("fn main() { if 1 { println(1) } }").unwrap();
        let error = run_main(&program).unwrap_err();

        assert!(
            error
                .message
                .contains("cannot assign integer literal to `bool`")
        );
    }

    #[test]
    fn rejects_bool_operator_with_integer() {
        let program = parse_source("fn main() { let value: bool = 1 && 2 }").unwrap();
        let error = run_main(&program).unwrap_err();

        assert!(
            error
                .message
                .contains("cannot assign integer literal to `bool`")
        );
    }

    #[test]
    fn rejects_equality_with_mismatched_types() {
        let program =
            parse_source("fn main() { let a: i32 = 1 let b: u32 = 1 println(a == b) }").unwrap();
        let error = run_main(&program).unwrap_err();

        assert!(error.message.contains("cannot use `u32` value as `i32`"));
    }

    #[test]
    fn runs_loop_until_break() {
        let program =
            parse_source("fn main() { loop { println(1) break println(2) } println(3) }").unwrap();

        assert_eq!(run_main(&program).unwrap(), "1\n3\n");
    }

    #[test]
    fn runs_while_until_break() {
        let program =
            parse_source("fn main() { while true { println(1) break println(2) } println(3) }")
                .unwrap();

        assert_eq!(run_main(&program).unwrap(), "1\n3\n");
    }

    #[test]
    fn skips_false_while_body() {
        let program = parse_source("fn main() { while false { println(1) } println(2) }").unwrap();

        assert_eq!(run_main(&program).unwrap(), "2\n");
    }

    #[test]
    fn returns_from_loop() {
        let program =
            parse_source("fn pick() -> i32 { loop { return 7 } } fn main() { println(pick()) }")
                .unwrap();

        assert_eq!(run_main(&program).unwrap(), "7\n");
    }

    #[test]
    fn rejects_break_outside_loop() {
        let program = parse_source("fn main() { break }").unwrap();
        let error = run_main(&program).unwrap_err();

        assert!(error.message.contains("cannot `break` outside loop"));
    }

    #[test]
    fn rejects_continue_outside_loop() {
        let program = parse_source("fn main() { continue }").unwrap();
        let error = run_main(&program).unwrap_err();

        assert!(error.message.contains("cannot `continue` outside loop"));
    }

    #[test]
    fn rejects_non_bool_while_condition() {
        let program = parse_source("fn main() { while 1 { break } }").unwrap();
        let error = run_main(&program).unwrap_err();

        assert!(
            error
                .message
                .contains("cannot assign integer literal to `bool`")
        );
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
    fn allocates_and_deallocates_value() {
        let program =
            parse_source("fn main() { let p: &i32 = alloc(7) println(*p) dealloc(p) }").unwrap();

        assert_eq!(run_main(&program).unwrap(), "7\n");
    }

    #[test]
    fn alloc_uses_expected_pointer_type() {
        let program =
            parse_source("fn main() { let p: &u8 = alloc(7) println(*p) dealloc(p) }").unwrap();

        assert_eq!(run_main(&program).unwrap(), "7\n");
    }

    #[test]
    fn allocates_struct() {
        let program = parse_source(
            "struct Point { x: i32 } fn main() { let p: &Point = alloc(Point { x: 9 }) println(p.x) dealloc(p) }",
        )
        .unwrap();

        assert_eq!(run_main(&program).unwrap(), "9\n");
    }

    #[test]
    fn rejects_use_after_dealloc() {
        let program =
            parse_source("fn main() { let p: &i32 = alloc(7) dealloc(p) println(*p) }").unwrap();
        let error = run_main(&program).unwrap_err();

        assert!(error.message.contains("freed pointer"));
    }

    #[test]
    fn rejects_dealloc_of_reference() {
        let program = parse_source("fn main() { let a: i32 = 7 dealloc(&a) }").unwrap();
        let error = run_main(&program).unwrap_err();

        assert!(error.message.contains("pointer returned by `alloc`"));
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
