use std::{
    collections::HashMap,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use frontend::{
    BinaryExpression, BinaryOperator, Block, CallExpression, CheckedProgram, Expression, Function,
    FunctionSignature, Program, Statement, StructLiteral, Type, check_program,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    pub message: String,
}

pub fn compile_file(source_path: &Path, output_path: &Path) -> Result<(), CompileError> {
    let source = fs::read_to_string(source_path).map_err(|error| CompileError {
        message: format!("failed to read `{}`: {error}", source_path.display()),
    })?;

    compile_source_to_executable(&source, output_path)
}

pub fn compile_file_to_assembly(
    source_path: &Path,
    output_path: &Path,
) -> Result<(), CompileError> {
    let source = fs::read_to_string(source_path).map_err(|error| CompileError {
        message: format!("failed to read `{}`: {error}", source_path.display()),
    })?;
    let assembly = compile_source_to_assembly(&source)?;

    fs::write(output_path, assembly).map_err(|error| CompileError {
        message: format!("failed to write `{}`: {error}", output_path.display()),
    })
}

pub fn compile_source_to_executable(source: &str, output_path: &Path) -> Result<(), CompileError> {
    let program = frontend::parse_source(source).map_err(|error| CompileError {
        message: format!(
            "parse error at {}..{}: {}",
            error.span.start, error.span.end, error.message
        ),
    })?;

    compile_program_to_executable(&program, output_path)
}

pub fn compile_source_to_assembly(source: &str) -> Result<String, CompileError> {
    let program = frontend::parse_source(source).map_err(|error| CompileError {
        message: format!(
            "parse error at {}..{}: {}",
            error.span.start, error.span.end, error.message
        ),
    })?;

    compile_program_to_assembly(&program)
}

pub fn compile_program_to_executable(
    program: &Program,
    output_path: &Path,
) -> Result<(), CompileError> {
    if !cfg!(target_os = "macos") || !cfg!(target_arch = "aarch64") {
        return Err(CompileError {
            message: "native backend currently supports only macOS arm64".to_string(),
        });
    }

    let assembly = compile_program_to_assembly(program)?;
    let assembly_path = temporary_assembly_path();

    fs::write(&assembly_path, assembly).map_err(|error| CompileError {
        message: format!("failed to write `{}`: {error}", assembly_path.display()),
    })?;

    let output = Command::new("cc")
        .arg(&assembly_path)
        .arg("-o")
        .arg(output_path)
        .output()
        .map_err(|error| CompileError {
            message: format!("failed to run `cc`: {error}"),
        })?;

    let _ = fs::remove_file(&assembly_path);

    if output.status.success() {
        return Ok(());
    }

    Err(CompileError {
        message: format!(
            "assembler/linker failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    })
}

pub fn compile_program_to_assembly(program: &Program) -> Result<String, CompileError> {
    let checked = check_program(program).map_err(|error| CompileError {
        message: error.message,
    })?;
    AssemblyCompiler::new(program, checked).compile()
}

#[derive(Debug, Clone)]
struct StructLayout {
    size: usize,
    fields: HashMap<String, FieldLayout>,
}

#[derive(Debug, Clone)]
struct FieldLayout {
    offset: usize,
    ty: Type,
}

struct AssemblyCompiler<'program> {
    program: &'program Program,
    checked: CheckedProgram,
    functions: HashMap<String, &'program Function>,
    layouts: HashMap<String, StructLayout>,
    next_label: usize,
}

impl<'program> AssemblyCompiler<'program> {
    fn new(program: &'program Program, checked: CheckedProgram) -> Self {
        let functions = program
            .functions
            .iter()
            .map(|function| (function.name.clone(), function))
            .collect();

        Self {
            program,
            checked,
            functions,
            layouts: HashMap::new(),
            next_label: 0,
        }
    }

    fn compile(mut self) -> Result<String, CompileError> {
        if !self.functions.contains_key("main") {
            return Err(compile_error("function `main` not found"));
        }

        self.layouts = build_struct_layouts(&self.checked)?;

        let mut text = String::new();

        for function in &self.program.functions {
            let function = FunctionCompiler::new(
                function,
                &self.checked.functions,
                &self.layouts,
                &mut self.next_label,
            )
            .compile()?;
            text.push_str(&function);
        }

        let mut assembly = String::new();
        assembly.push_str(".build_version macos, 11, 0\n");
        assembly.push_str(".section __TEXT,__cstring,cstring_literals\n");
        assembly.push_str("L_mould_fmt_signed:\n    .asciz \"%ld\\n\"\n");
        assembly.push_str("L_mould_fmt_unsigned:\n    .asciz \"%lu\\n\"\n");
        assembly.push_str("L_mould_fmt_float:\n    .asciz \"%f\\n\"\n");
        assembly.push_str("L_mould_fmt_pointer:\n    .asciz \"%p\\n\"\n");
        assembly.push_str("L_mould_bool_true:\n    .asciz \"true\"\n");
        assembly.push_str("L_mould_bool_false:\n    .asciz \"false\"\n");
        assembly.push('\n');
        assembly.push_str(".section __TEXT,__text,regular,pure_instructions\n\n");
        assembly.push_str(&text);
        assembly.push_str(U128_PRINT_HELPERS);
        assembly.push_str(".subsections_via_symbols\n");

        Ok(assembly)
    }
}

#[derive(Debug, Clone)]
struct Local {
    ty: Type,
    offset: usize,
    storage: LocalStorage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalStorage {
    Direct,
    Indirect,
}

#[derive(Debug, Clone)]
struct LoopLabels {
    continue_label: String,
    break_label: String,
}

struct FunctionCompiler<'program, 'shared> {
    function: &'program Function,
    signatures: &'shared HashMap<String, FunctionSignature>,
    layouts: &'shared HashMap<String, StructLayout>,
    next_label: &'shared mut usize,
    body: String,
    scopes: Vec<HashMap<String, Local>>,
    loop_stack: Vec<LoopLabels>,
    next_offset: usize,
    return_label: String,
}

impl<'program, 'shared> FunctionCompiler<'program, 'shared> {
    fn new(
        function: &'program Function,
        signatures: &'shared HashMap<String, FunctionSignature>,
        layouts: &'shared HashMap<String, StructLayout>,
        next_label: &'shared mut usize,
    ) -> Self {
        let return_label = format!("L_mould_{}_return", function.name);

        Self {
            function,
            signatures,
            layouts,
            next_label,
            body: String::new(),
            scopes: Vec::new(),
            loop_stack: Vec::new(),
            next_offset: 0,
            return_label,
        }
    }

    fn compile(mut self) -> Result<String, CompileError> {
        if self.function.name == "main" && !self.function.parameters.is_empty() {
            return Err(compile_error("function `main` cannot have parameters"));
        }

        if let Some(return_type) = &self.function.return_type {
            ensure_supported_type(return_type)?;
        }

        self.scopes.push(HashMap::new());

        let mut argument_register = 0;

        for parameter in &self.function.parameters {
            ensure_supported_type(&parameter.ty)?;
            let register_count = value_register_count(&parameter.ty);
            if argument_register + register_count > 8 {
                return Err(compile_error(
                    "native backend supports up to 8 argument registers",
                ));
            }

            let storage = if matches!(parameter.ty, Type::Struct(_)) {
                LocalStorage::Indirect
            } else {
                LocalStorage::Direct
            };
            let local = self.allocate_local(parameter.ty.clone(), storage)?;
            self.insert_local(parameter.name.clone(), local.clone())?;

            if is_128_integer(&parameter.ty) {
                self.store_register_pair_to_frame(
                    &format!("x{argument_register}"),
                    &format!("x{}", argument_register + 1),
                    local.offset,
                )?;
            } else {
                self.store_register_to_frame(&format!("x{argument_register}"), local.offset)?;
            }
            argument_register += register_count;
        }

        self.compile_block(&self.function.body)?;

        let frame_size = align_to(self.next_offset, 16);
        let label = function_label(&self.function.name);
        let mut text = String::new();

        writeln!(text, ".globl {label}").unwrap();
        text.push_str(".p2align 2\n");
        writeln!(text, "{label}:").unwrap();
        text.push_str("    stp x29, x30, [sp, #-16]!\n");
        text.push_str("    mov x29, sp\n");
        if frame_size > 0 {
            emit_sub_imm(&mut text, "sp", "sp", frame_size);
        }
        text.push_str(&self.body);
        writeln!(text, "{}:", self.return_label).unwrap();
        if self.function.return_type.is_none() {
            text.push_str("    mov x0, #0\n");
        }
        if frame_size > 0 {
            emit_add_imm(&mut text, "sp", "sp", frame_size);
        }
        text.push_str("    ldp x29, x30, [sp], #16\n");
        text.push_str("    ret\n\n");

        Ok(text)
    }

    fn compile_block(&mut self, block: &Block) -> Result<(), CompileError> {
        self.scopes.push(HashMap::new());

        for statement in &block.statements {
            self.compile_statement(statement)?;
        }

        self.scopes.pop();
        Ok(())
    }

    fn compile_statement(&mut self, statement: &Statement) -> Result<(), CompileError> {
        match statement {
            Statement::Let(statement) => {
                ensure_supported_type(&statement.ty)?;
                let local = self.allocate_local(statement.ty.clone(), LocalStorage::Direct)?;
                self.store_expression_to_local(&statement.value, &statement.ty, &local)?;
                self.insert_local(statement.name.clone(), local)
            }
            Statement::Call(statement) => {
                let call = CallExpression {
                    name: statement.name.clone(),
                    arguments: statement.arguments.clone(),
                };
                self.compile_call_statement(&call)
            }
            Statement::Return(statement) => {
                let Some(return_type) = self.function.return_type.clone() else {
                    return Err(compile_error(
                        "cannot return a value from function without return type",
                    ));
                };

                ensure_supported_type(&return_type)?;
                if matches!(return_type, Type::Struct(_)) {
                    return Err(compile_error("native backend cannot return structs yet"));
                }

                self.compile_expression(&statement.value, Some(&return_type))?;
                writeln!(self.body, "    b {}", self.return_label).unwrap();
                Ok(())
            }
            Statement::If(statement) => {
                let else_label = self.make_label("if_else");
                let end_label = self.make_label("if_end");

                self.compile_expression(&statement.condition, Some(&Type::Bool))?;
                self.body.push_str("    cmp x0, #0\n");
                writeln!(self.body, "    b.eq {else_label}").unwrap();
                self.compile_block(&statement.then_block)?;
                writeln!(self.body, "    b {end_label}").unwrap();
                writeln!(self.body, "{else_label}:").unwrap();
                if let Some(block) = &statement.else_block {
                    self.compile_block(block)?;
                }
                writeln!(self.body, "{end_label}:").unwrap();
                Ok(())
            }
            Statement::Loop(statement) => {
                let start_label = self.make_label("loop_start");
                let end_label = self.make_label("loop_end");

                self.loop_stack.push(LoopLabels {
                    continue_label: start_label.clone(),
                    break_label: end_label.clone(),
                });
                writeln!(self.body, "{start_label}:").unwrap();
                self.compile_block(&statement.body)?;
                writeln!(self.body, "    b {start_label}").unwrap();
                writeln!(self.body, "{end_label}:").unwrap();
                self.loop_stack.pop();
                Ok(())
            }
            Statement::While(statement) => {
                let start_label = self.make_label("while_start");
                let end_label = self.make_label("while_end");

                self.loop_stack.push(LoopLabels {
                    continue_label: start_label.clone(),
                    break_label: end_label.clone(),
                });
                writeln!(self.body, "{start_label}:").unwrap();
                self.compile_expression(&statement.condition, Some(&Type::Bool))?;
                self.body.push_str("    cmp x0, #0\n");
                writeln!(self.body, "    b.eq {end_label}").unwrap();
                self.compile_block(&statement.body)?;
                writeln!(self.body, "    b {start_label}").unwrap();
                writeln!(self.body, "{end_label}:").unwrap();
                self.loop_stack.pop();
                Ok(())
            }
            Statement::Break => {
                let Some(labels) = self.loop_stack.last() else {
                    return Err(compile_error("cannot `break` outside loop"));
                };
                writeln!(self.body, "    b {}", labels.break_label).unwrap();
                Ok(())
            }
            Statement::Continue => {
                let Some(labels) = self.loop_stack.last() else {
                    return Err(compile_error("cannot `continue` outside loop"));
                };
                writeln!(self.body, "    b {}", labels.continue_label).unwrap();
                Ok(())
            }
        }
    }

    fn compile_call_statement(&mut self, call: &CallExpression) -> Result<(), CompileError> {
        match call.name.as_str() {
            "println" => self.compile_println(call),
            "dealloc" => self.compile_dealloc(call),
            "alloc" => {
                self.compile_alloc(call, None)?;
                Ok(())
            }
            _ => {
                self.compile_user_call(call)?;
                Ok(())
            }
        }
    }

    fn compile_expression(
        &mut self,
        expression: &Expression,
        expected: Option<&Type>,
    ) -> Result<Type, CompileError> {
        let ty = self.expression_type(expression, expected)?;
        ensure_supported_type(&ty)?;

        match expression {
            Expression::Integer(value) => {
                self.compile_integer_literal(*value, &ty);
                Ok(ty)
            }
            Expression::Float(value) => {
                self.compile_float_literal(value, &ty)?;
                Ok(ty)
            }
            Expression::Bool(value) => {
                writeln!(self.body, "    mov x0, #{}", if *value { 1 } else { 0 }).unwrap();
                Ok(Type::Bool)
            }
            Expression::Variable(name) => {
                let local = self.local(name)?;
                if matches!(local.ty, Type::Struct(_)) {
                    self.local_address(&local, "x0")?;
                } else {
                    self.load_local_to_x0(&local)?;
                }
                Ok(ty)
            }
            Expression::Call(call) => match call.name.as_str() {
                "alloc" => self.compile_alloc(call, expected),
                "println" => Err(compile_error("function `println` does not return a value")),
                "dealloc" => Err(compile_error("function `dealloc` does not return a value")),
                _ => self.compile_user_call(call)?.ok_or_else(|| {
                    compile_error(format!("function `{}` does not return a value", call.name))
                }),
            },
            Expression::StructLiteral(literal) => {
                let local = self.allocate_local(ty.clone(), LocalStorage::Direct)?;
                self.compile_struct_literal_to_local(literal, &local)?;
                self.local_address(&local, "x0")?;
                Ok(ty)
            }
            Expression::FieldAccess(access) => {
                let field_ty = self.compile_field_address(access)?;
                if matches!(field_ty, Type::Struct(_)) {
                    Ok(field_ty)
                } else {
                    self.load_address_to_x0(&field_ty)?;
                    Ok(field_ty)
                }
            }
            Expression::AddressOf(expression) => {
                let inner = self.compile_lvalue_address(expression)?;
                Ok(Type::Pointer(Box::new(inner)))
            }
            Expression::Dereference(expression) => {
                let pointer_ty = self.compile_expression(expression, None)?;
                let Type::Pointer(inner) = pointer_ty else {
                    return Err(compile_error(format!(
                        "cannot dereference `{}`",
                        pointer_ty.name()
                    )));
                };

                if matches!(*inner, Type::Struct(_)) {
                    Ok(*inner)
                } else {
                    self.load_address_to_x0(&inner)?;
                    Ok(*inner)
                }
            }
            Expression::BitNot(expression) => {
                self.compile_expression(expression, Some(&ty))?;

                if ty.is_bool() {
                    self.body.push_str("    cmp x0, #0\n");
                    self.body.push_str("    cset x0, eq\n");
                } else if is_128_integer(&ty) {
                    self.body.push_str("    mvn x0, x0\n");
                    self.body.push_str("    mvn x1, x1\n");
                } else {
                    self.body.push_str("    mvn x0, x0\n");
                    self.normalize_x0(&ty);
                }

                Ok(ty)
            }
            Expression::Binary(binary) => {
                self.compile_binary_expression(binary, &ty)?;
                Ok(ty)
            }
        }
    }

    fn compile_binary_expression(
        &mut self,
        binary: &BinaryExpression,
        result_ty: &Type,
    ) -> Result<(), CompileError> {
        match binary.operator {
            BinaryOperator::BoolAnd => self.compile_bool_and(binary),
            BinaryOperator::BoolOr => self.compile_bool_or(binary),
            BinaryOperator::Equal | BinaryOperator::NotEqual => self.compile_equality(binary),
            _ => self.compile_value_binary(binary, result_ty),
        }
    }

    fn compile_bool_and(&mut self, binary: &BinaryExpression) -> Result<(), CompileError> {
        let false_label = self.make_label("bool_and_false");
        let end_label = self.make_label("bool_and_end");

        self.compile_expression(&binary.left, Some(&Type::Bool))?;
        self.body.push_str("    cmp x0, #0\n");
        writeln!(self.body, "    b.eq {false_label}").unwrap();
        self.compile_expression(&binary.right, Some(&Type::Bool))?;
        self.body.push_str("    cmp x0, #0\n");
        self.body.push_str("    cset x0, ne\n");
        writeln!(self.body, "    b {end_label}").unwrap();
        writeln!(self.body, "{false_label}:").unwrap();
        self.body.push_str("    mov x0, #0\n");
        writeln!(self.body, "{end_label}:").unwrap();
        Ok(())
    }

    fn compile_bool_or(&mut self, binary: &BinaryExpression) -> Result<(), CompileError> {
        let true_label = self.make_label("bool_or_true");
        let end_label = self.make_label("bool_or_end");

        self.compile_expression(&binary.left, Some(&Type::Bool))?;
        self.body.push_str("    cmp x0, #0\n");
        writeln!(self.body, "    b.ne {true_label}").unwrap();
        self.compile_expression(&binary.right, Some(&Type::Bool))?;
        self.body.push_str("    cmp x0, #0\n");
        self.body.push_str("    cset x0, ne\n");
        writeln!(self.body, "    b {end_label}").unwrap();
        writeln!(self.body, "{true_label}:").unwrap();
        self.body.push_str("    mov x0, #1\n");
        writeln!(self.body, "{end_label}:").unwrap();
        Ok(())
    }

    fn compile_equality(&mut self, binary: &BinaryExpression) -> Result<(), CompileError> {
        let operand_ty = self.binary_operand_type(binary, None)?;

        if matches!(operand_ty, Type::Struct(_)) {
            return Err(compile_error("native backend cannot compare structs yet"));
        }

        if is_128_integer(&operand_ty) {
            self.compile_expression(&binary.left, Some(&operand_ty))?;
            self.push_value(&operand_ty);
            self.compile_expression(&binary.right, Some(&operand_ty))?;
            self.body.push_str("    mov x2, x0\n");
            self.body.push_str("    mov x3, x1\n");
            self.pop_value_to_x0_x1(&operand_ty);
            self.body.push_str("    eor x0, x0, x2\n");
            self.body.push_str("    eor x1, x1, x3\n");
            self.body.push_str("    orr x0, x0, x1\n");
            self.body.push_str("    cmp x0, #0\n");
            let condition = if binary.operator == BinaryOperator::Equal {
                "eq"
            } else {
                "ne"
            };
            writeln!(self.body, "    cset x0, {condition}").unwrap();
            return Ok(());
        }

        self.compile_expression(&binary.left, Some(&operand_ty))?;
        self.push_value(&operand_ty);
        self.compile_expression(&binary.right, Some(&operand_ty))?;
        self.body.push_str("    mov x1, x0\n");
        self.pop_value_to_x0_x1(&operand_ty);

        match operand_ty {
            Type::F16 | Type::F32 => {
                self.body.push_str("    fmov s0, w0\n");
                self.body.push_str("    fmov s1, w1\n");
                self.body.push_str("    fcmp s0, s1\n");
            }
            Type::F64 => {
                self.body.push_str("    fmov d0, x0\n");
                self.body.push_str("    fmov d1, x1\n");
                self.body.push_str("    fcmp d0, d1\n");
            }
            _ => self.body.push_str("    cmp x0, x1\n"),
        }

        let condition = if binary.operator == BinaryOperator::Equal {
            "eq"
        } else {
            "ne"
        };
        writeln!(self.body, "    cset x0, {condition}").unwrap();
        Ok(())
    }

    fn compile_value_binary(
        &mut self,
        binary: &BinaryExpression,
        ty: &Type,
    ) -> Result<(), CompileError> {
        self.compile_expression(&binary.left, Some(ty))?;
        self.push_value(ty);
        self.compile_expression(&binary.right, Some(ty))?;

        if is_128_integer(ty) {
            self.body.push_str("    mov x2, x0\n");
            self.body.push_str("    mov x3, x1\n");
            self.pop_value_to_x0_x1(ty);
            return self.compile_integer128_binary_operator(binary.operator, ty);
        }

        self.body.push_str("    mov x1, x0\n");
        self.pop_value_to_x0_x1(ty);

        if ty.is_float() {
            self.compile_float_binary_operator(binary.operator, ty)?;
        } else {
            self.compile_integer_binary_operator(binary.operator, ty)?;
        }

        Ok(())
    }

    fn compile_integer_binary_operator(
        &mut self,
        operator: BinaryOperator,
        ty: &Type,
    ) -> Result<(), CompileError> {
        match operator {
            BinaryOperator::Add => self.body.push_str("    add x0, x0, x1\n"),
            BinaryOperator::Subtract => self.body.push_str("    sub x0, x0, x1\n"),
            BinaryOperator::Multiply => self.body.push_str("    mul x0, x0, x1\n"),
            BinaryOperator::Divide if ty.is_signed_integer() => {
                self.body.push_str("    sdiv x0, x0, x1\n");
            }
            BinaryOperator::Divide => self.body.push_str("    udiv x0, x0, x1\n"),
            BinaryOperator::BitAnd => self.body.push_str("    and x0, x0, x1\n"),
            BinaryOperator::BitOr => self.body.push_str("    orr x0, x0, x1\n"),
            BinaryOperator::BitXor => self.body.push_str("    eor x0, x0, x1\n"),
            BinaryOperator::ShiftLeft => self.body.push_str("    lsl x0, x0, x1\n"),
            BinaryOperator::ShiftRight if ty.is_signed_integer() => {
                self.body.push_str("    asr x0, x0, x1\n");
            }
            BinaryOperator::ShiftRight => self.body.push_str("    lsr x0, x0, x1\n"),
            _ => {
                return Err(compile_error(format!(
                    "operator `{}` cannot be used with `{}`",
                    operator.symbol(),
                    ty.name()
                )));
            }
        }

        self.normalize_x0(ty);
        Ok(())
    }

    fn compile_integer128_binary_operator(
        &mut self,
        operator: BinaryOperator,
        ty: &Type,
    ) -> Result<(), CompileError> {
        match operator {
            BinaryOperator::Add => {
                self.body.push_str("    adds x0, x0, x2\n");
                self.body.push_str("    adc x1, x1, x3\n");
            }
            BinaryOperator::Subtract => {
                self.body.push_str("    subs x0, x0, x2\n");
                self.body.push_str("    sbc x1, x1, x3\n");
            }
            BinaryOperator::Multiply => {
                self.body.push_str("    mul x4, x0, x2\n");
                self.body.push_str("    umulh x5, x0, x2\n");
                self.body.push_str("    madd x5, x0, x3, x5\n");
                self.body.push_str("    madd x1, x1, x2, x5\n");
                self.body.push_str("    mov x0, x4\n");
            }
            BinaryOperator::BitAnd => {
                self.body.push_str("    and x0, x0, x2\n");
                self.body.push_str("    and x1, x1, x3\n");
            }
            BinaryOperator::BitOr => {
                self.body.push_str("    orr x0, x0, x2\n");
                self.body.push_str("    orr x1, x1, x3\n");
            }
            BinaryOperator::BitXor => {
                self.body.push_str("    eor x0, x0, x2\n");
                self.body.push_str("    eor x1, x1, x3\n");
            }
            BinaryOperator::Divide if ty.is_signed_integer() => {
                self.compile_integer128_signed_division()
            }
            BinaryOperator::Divide => {
                self.body.push_str("    bl L_mould_udivmod_u128\n");
            }
            BinaryOperator::ShiftLeft => self.compile_integer128_shift_left(),
            BinaryOperator::ShiftRight => {
                self.compile_integer128_shift_right(ty.is_signed_integer())
            }
            _ => {
                return Err(compile_error(format!(
                    "operator `{}` cannot be used with `{}`",
                    operator.symbol(),
                    ty.name()
                )));
            }
        }

        Ok(())
    }

    fn compile_integer128_signed_division(&mut self) {
        let left_positive_label = self.make_label("i128_div_left_positive");
        let right_positive_label = self.make_label("i128_div_right_positive");
        let positive_result_label = self.make_label("i128_div_positive_result");
        let end_label = self.make_label("i128_div_end");

        self.body.push_str("    eor x4, x1, x3\n");
        self.body.push_str("    lsr x4, x4, #63\n");
        writeln!(self.body, "    tbz x1, #63, {left_positive_label}").unwrap();
        self.body.push_str("    mvn x0, x0\n");
        self.body.push_str("    mvn x1, x1\n");
        self.body.push_str("    adds x0, x0, #1\n");
        self.body.push_str("    adc x1, x1, xzr\n");
        writeln!(self.body, "{left_positive_label}:").unwrap();
        writeln!(self.body, "    tbz x3, #63, {right_positive_label}").unwrap();
        self.body.push_str("    mvn x2, x2\n");
        self.body.push_str("    mvn x3, x3\n");
        self.body.push_str("    adds x2, x2, #1\n");
        self.body.push_str("    adc x3, x3, xzr\n");
        writeln!(self.body, "{right_positive_label}:").unwrap();
        self.body.push_str("    sub sp, sp, #16\n");
        self.body.push_str("    str x4, [sp]\n");
        self.body.push_str("    bl L_mould_udivmod_u128\n");
        self.body.push_str("    ldr x4, [sp]\n");
        self.body.push_str("    add sp, sp, #16\n");
        writeln!(self.body, "    cbz x4, {positive_result_label}").unwrap();
        self.body.push_str("    mvn x0, x0\n");
        self.body.push_str("    mvn x1, x1\n");
        self.body.push_str("    adds x0, x0, #1\n");
        self.body.push_str("    adc x1, x1, xzr\n");
        writeln!(self.body, "    b {end_label}").unwrap();
        writeln!(self.body, "{positive_result_label}:").unwrap();
        writeln!(self.body, "{end_label}:").unwrap();
    }

    fn compile_integer128_shift_left(&mut self) {
        let ge64_label = self.make_label("shl128_ge64");
        let end_label = self.make_label("shl128_end");

        self.body.push_str("    and x2, x2, #0x7f\n");
        self.body.push_str("    cmp x2, #64\n");
        writeln!(self.body, "    b.hs {ge64_label}").unwrap();
        self.body.push_str("    cmp x2, #0\n");
        writeln!(self.body, "    b.eq {end_label}").unwrap();
        self.body.push_str("    mov x4, x1\n");
        self.body.push_str("    lsl x1, x1, x2\n");
        self.body.push_str("    mov x5, #64\n");
        self.body.push_str("    sub x5, x5, x2\n");
        self.body.push_str("    lsr x4, x0, x5\n");
        self.body.push_str("    orr x1, x1, x4\n");
        self.body.push_str("    lsl x0, x0, x2\n");
        writeln!(self.body, "    b {end_label}").unwrap();
        writeln!(self.body, "{ge64_label}:").unwrap();
        self.body.push_str("    sub x2, x2, #64\n");
        self.body.push_str("    lsl x1, x0, x2\n");
        self.body.push_str("    mov x0, #0\n");
        writeln!(self.body, "{end_label}:").unwrap();
    }

    fn compile_integer128_shift_right(&mut self, signed: bool) {
        let ge64_label = self.make_label("shr128_ge64");
        let end_label = self.make_label("shr128_end");

        self.body.push_str("    and x2, x2, #0x7f\n");
        self.body.push_str("    cmp x2, #64\n");
        writeln!(self.body, "    b.hs {ge64_label}").unwrap();
        self.body.push_str("    cmp x2, #0\n");
        writeln!(self.body, "    b.eq {end_label}").unwrap();
        self.body.push_str("    mov x4, x0\n");
        self.body.push_str("    lsr x0, x0, x2\n");
        self.body.push_str("    mov x5, #64\n");
        self.body.push_str("    sub x5, x5, x2\n");
        self.body.push_str("    lsl x4, x1, x5\n");
        self.body.push_str("    orr x0, x0, x4\n");
        if signed {
            self.body.push_str("    asr x1, x1, x2\n");
        } else {
            self.body.push_str("    lsr x1, x1, x2\n");
        }
        writeln!(self.body, "    b {end_label}").unwrap();
        writeln!(self.body, "{ge64_label}:").unwrap();
        self.body.push_str("    sub x2, x2, #64\n");
        if signed {
            self.body.push_str("    asr x0, x1, x2\n");
            self.body.push_str("    asr x1, x1, #63\n");
        } else {
            self.body.push_str("    lsr x0, x1, x2\n");
            self.body.push_str("    mov x1, #0\n");
        }
        writeln!(self.body, "{end_label}:").unwrap();
    }

    fn compile_float_binary_operator(
        &mut self,
        operator: BinaryOperator,
        ty: &Type,
    ) -> Result<(), CompileError> {
        match ty {
            Type::F16 | Type::F32 => {
                self.body.push_str("    fmov s0, w0\n");
                self.body.push_str("    fmov s1, w1\n");
                match operator {
                    BinaryOperator::Add => self.body.push_str("    fadd s0, s0, s1\n"),
                    BinaryOperator::Subtract => self.body.push_str("    fsub s0, s0, s1\n"),
                    BinaryOperator::Multiply => self.body.push_str("    fmul s0, s0, s1\n"),
                    BinaryOperator::Divide => self.body.push_str("    fdiv s0, s0, s1\n"),
                    _ => {
                        return Err(compile_error(format!(
                            "operator `{}` cannot be used with `{}`",
                            operator.symbol(),
                            ty.name()
                        )));
                    }
                }
                self.body.push_str("    fmov w0, s0\n");
            }
            Type::F64 => {
                self.body.push_str("    fmov d0, x0\n");
                self.body.push_str("    fmov d1, x1\n");
                match operator {
                    BinaryOperator::Add => self.body.push_str("    fadd d0, d0, d1\n"),
                    BinaryOperator::Subtract => self.body.push_str("    fsub d0, d0, d1\n"),
                    BinaryOperator::Multiply => self.body.push_str("    fmul d0, d0, d1\n"),
                    BinaryOperator::Divide => self.body.push_str("    fdiv d0, d0, d1\n"),
                    _ => {
                        return Err(compile_error(format!(
                            "operator `{}` cannot be used with `{}`",
                            operator.symbol(),
                            ty.name()
                        )));
                    }
                }
                self.body.push_str("    fmov x0, d0\n");
            }
            _ => unreachable!("float operator checked by type checker"),
        }

        Ok(())
    }

    fn compile_user_call(&mut self, call: &CallExpression) -> Result<Option<Type>, CompileError> {
        let signature = self
            .signatures
            .get(&call.name)
            .ok_or_else(|| compile_error(format!("unknown function `{}`", call.name)))?
            .clone();

        if signature.parameters.len() != call.arguments.len() {
            return Err(compile_error(format!(
                "function `{}` expects {} argument(s)",
                call.name,
                signature.parameters.len()
            )));
        }

        let register_count = signature
            .parameters
            .iter()
            .map(value_register_count)
            .sum::<usize>();
        if register_count > 8 {
            return Err(compile_error(
                "native backend supports up to 8 argument registers",
            ));
        }

        for (argument, parameter_ty) in call.arguments.iter().zip(&signature.parameters).rev() {
            ensure_supported_type(parameter_ty)?;
            self.compile_expression(argument, Some(parameter_ty))?;
            self.push_value(parameter_ty);
        }

        let mut stack_offset = 0;
        let mut register = 0;
        for parameter_ty in &signature.parameters {
            if is_128_integer(parameter_ty) {
                writeln!(self.body, "    ldr x{register}, [sp, #{stack_offset}]").unwrap();
                writeln!(
                    self.body,
                    "    ldr x{}, [sp, #{}]",
                    register + 1,
                    stack_offset + 8
                )
                .unwrap();
                register += 2;
            } else {
                writeln!(self.body, "    ldr x{register}, [sp, #{stack_offset}]").unwrap();
                register += 1;
            }
            stack_offset += 16;
        }

        if !signature.parameters.is_empty() {
            emit_add_imm(&mut self.body, "sp", "sp", signature.parameters.len() * 16);
        }

        writeln!(self.body, "    bl {}", function_label(&call.name)).unwrap();

        let Some(return_type) = signature.return_type else {
            return Ok(None);
        };

        if matches!(return_type, Type::Struct(_)) {
            return Err(compile_error("native backend cannot return structs yet"));
        }

        self.normalize_x0(&return_type);
        Ok(Some(return_type))
    }

    fn compile_println(&mut self, call: &CallExpression) -> Result<(), CompileError> {
        if call.arguments.len() != 1 {
            return Err(compile_error("function `println` expects 1 argument"));
        }

        let ty = self.compile_expression(&call.arguments[0], None)?;

        match ty {
            Type::Bool => {
                let false_label = self.make_label("println_false");
                let end_label = self.make_label("println_end");
                self.body.push_str("    cmp x0, #0\n");
                writeln!(self.body, "    b.eq {false_label}").unwrap();
                emit_load_label_address(&mut self.body, "x0", "L_mould_bool_true");
                self.body.push_str("    bl _puts\n");
                writeln!(self.body, "    b {end_label}").unwrap();
                writeln!(self.body, "{false_label}:").unwrap();
                emit_load_label_address(&mut self.body, "x0", "L_mould_bool_false");
                self.body.push_str("    bl _puts\n");
                writeln!(self.body, "{end_label}:").unwrap();
            }
            Type::Pointer(_) => {
                self.emit_printf_with_x0("L_mould_fmt_pointer");
            }
            Type::I128 => {
                self.body.push_str("    bl _mould_print_i128\n");
            }
            Type::U128 => {
                self.body.push_str("    bl _mould_print_u128\n");
            }
            Type::F16 | Type::F32 => {
                self.body.push_str("    fmov s0, w0\n");
                self.body.push_str("    fcvt d0, s0\n");
                self.body.push_str("    fmov x0, d0\n");
                self.emit_printf_with_x0("L_mould_fmt_float");
            }
            Type::F64 => {
                self.emit_printf_with_x0("L_mould_fmt_float");
            }
            ty if ty.is_signed_integer() => {
                self.emit_printf_with_x0("L_mould_fmt_signed");
            }
            ty if ty.is_integer() => {
                self.emit_printf_with_x0("L_mould_fmt_unsigned");
            }
            Type::Struct(_) => {
                return Err(compile_error("native backend cannot print structs yet"));
            }
            ty => {
                return Err(compile_error(format!(
                    "native backend cannot print `{}` yet",
                    ty.name()
                )));
            }
        }

        Ok(())
    }

    fn compile_alloc(
        &mut self,
        call: &CallExpression,
        expected: Option<&Type>,
    ) -> Result<Type, CompileError> {
        if call.arguments.len() != 1 {
            return Err(compile_error("function `alloc` expects 1 argument"));
        }

        let pointer_ty = self.expression_type(&Expression::Call(call.clone()), expected)?;
        let Type::Pointer(inner) = pointer_ty.clone() else {
            return Err(compile_error("function `alloc` returns a pointer"));
        };
        ensure_supported_type(&inner)?;

        let size = self.type_size(&inner)?;
        emit_mov_u64(&mut self.body, "x0", size as u64);
        self.body.push_str("    bl _malloc\n");

        let pointer_slot = self.allocate_temp_slot()?;
        self.store_register_to_frame("x0", pointer_slot)?;

        if matches!(*inner, Type::Struct(_)) {
            self.compile_expression(&call.arguments[0], Some(&inner))?;
            self.load_frame_to_register(pointer_slot, "x9")?;
            self.copy_value_to_address(&inner, "x0", "x9")?;
        } else {
            self.compile_expression(&call.arguments[0], Some(&inner))?;
            self.load_frame_to_register(pointer_slot, "x9")?;
            self.store_value_to_address(&inner, "x9")?;
        }

        self.load_frame_to_register(pointer_slot, "x0")?;
        Ok(pointer_ty)
    }

    fn compile_dealloc(&mut self, call: &CallExpression) -> Result<(), CompileError> {
        if call.arguments.len() != 1 {
            return Err(compile_error("function `dealloc` expects 1 argument"));
        }

        let ty = self.compile_expression(&call.arguments[0], None)?;
        if !matches!(ty, Type::Pointer(_)) {
            return Err(compile_error(format!(
                "function `dealloc` expects pointer, found `{}`",
                ty.name()
            )));
        }

        self.body.push_str("    bl _free\n");
        Ok(())
    }

    fn store_expression_to_local(
        &mut self,
        expression: &Expression,
        ty: &Type,
        local: &Local,
    ) -> Result<(), CompileError> {
        if matches!(ty, Type::Struct(_)) {
            if let Expression::StructLiteral(literal) = expression {
                return self.compile_struct_literal_to_local(literal, local);
            }

            self.compile_expression(expression, Some(ty))?;
            self.local_address(local, "x9")?;
            self.copy_value_to_address(ty, "x0", "x9")?;
        } else {
            self.compile_expression(expression, Some(ty))?;
            self.store_value_to_local(ty, local)?;
        }

        Ok(())
    }

    fn compile_struct_literal_to_local(
        &mut self,
        literal: &StructLiteral,
        local: &Local,
    ) -> Result<(), CompileError> {
        let layout = self.struct_layout(&literal.name)?.clone();

        for (field_name, field_layout) in &layout.fields {
            let field = literal
                .fields
                .iter()
                .find(|field| field.name == *field_name)
                .ok_or_else(|| {
                    compile_error(format!(
                        "missing field `{field_name}` in `{}`",
                        literal.name
                    ))
                })?;

            if matches!(field_layout.ty, Type::Struct(_)) {
                self.compile_expression(&field.value, Some(&field_layout.ty))?;
                self.local_field_address(local, field_layout.offset, "x9")?;
                self.copy_value_to_address(&field_layout.ty, "x0", "x9")?;
            } else {
                self.compile_expression(&field.value, Some(&field_layout.ty))?;
                self.local_field_address(local, field_layout.offset, "x9")?;
                self.store_value_to_address(&field_layout.ty, "x9")?;
            }
        }

        Ok(())
    }

    fn compile_lvalue_address(&mut self, expression: &Expression) -> Result<Type, CompileError> {
        match expression {
            Expression::Variable(name) => {
                let local = self.local(name)?;
                self.local_address(&local, "x0")?;
                Ok(local.ty)
            }
            Expression::FieldAccess(access) => self.compile_field_address(access),
            Expression::Dereference(expression) => {
                let pointer_ty = self.compile_expression(expression, None)?;
                let Type::Pointer(inner) = pointer_ty else {
                    return Err(compile_error(format!(
                        "cannot dereference `{}`",
                        pointer_ty.name()
                    )));
                };
                Ok(*inner)
            }
            _ => Err(compile_error("cannot take address of temporary value")),
        }
    }

    fn compile_field_address(
        &mut self,
        access: &frontend::FieldAccess,
    ) -> Result<Type, CompileError> {
        let object_ty = self.expression_type(&access.object, None)?;
        let struct_name = match object_ty {
            Type::Struct(name) => {
                self.compile_lvalue_address(&access.object)?;
                name
            }
            Type::Pointer(inner) => match *inner {
                Type::Struct(name) => {
                    self.compile_expression(&access.object, None)?;
                    name
                }
                ty => {
                    return Err(compile_error(format!(
                        "cannot access field `{}` on `&{}`",
                        access.field,
                        ty.name()
                    )));
                }
            },
            ty => {
                return Err(compile_error(format!(
                    "cannot access field `{}` on `{}`",
                    access.field,
                    ty.name()
                )));
            }
        };

        let field = self
            .struct_layout(&struct_name)?
            .fields
            .get(&access.field)
            .cloned()
            .ok_or_else(|| {
                compile_error(format!(
                    "unknown field `{}` in `{struct_name}`",
                    access.field
                ))
            })?;
        emit_add_imm(&mut self.body, "x0", "x0", field.offset);
        Ok(field.ty)
    }

    fn expression_type(
        &self,
        expression: &Expression,
        expected: Option<&Type>,
    ) -> Result<Type, CompileError> {
        match expression {
            Expression::Integer(value) => {
                let ty = expected.cloned().unwrap_or(Type::I32);
                if !ty.is_integer() {
                    return Err(compile_error(format!(
                        "cannot assign integer literal to `{}`",
                        ty.name()
                    )));
                }
                if value > &ty.max_integer_value().expect("integer type has max value") {
                    return Err(compile_error(format!(
                        "integer literal `{value}` does not fit in `{}`",
                        ty.name()
                    )));
                }
                Ok(ty)
            }
            Expression::Float(_) => {
                let ty = expected.cloned().unwrap_or(Type::F64);
                if ty.is_float() {
                    Ok(ty)
                } else {
                    Err(compile_error(format!(
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
                let ty = self.local(name)?.ty;
                if let Some(expected) = expected {
                    expect_type(&ty, expected)
                } else {
                    Ok(ty)
                }
            }
            Expression::Call(call) => match call.name.as_str() {
                "alloc" => {
                    if let Some(Type::Pointer(inner)) = expected {
                        Ok(Type::Pointer(inner.clone()))
                    } else if expected.is_some() {
                        Err(compile_error("function `alloc` returns a pointer"))
                    } else {
                        Ok(Type::Pointer(Box::new(
                            self.expression_type(&call.arguments[0], None)?,
                        )))
                    }
                }
                "println" => Err(compile_error("function `println` does not return a value")),
                "dealloc" => Err(compile_error("function `dealloc` does not return a value")),
                name => {
                    let signature = self
                        .signatures
                        .get(name)
                        .ok_or_else(|| compile_error(format!("unknown function `{name}`")))?;
                    let Some(return_type) = &signature.return_type else {
                        return Err(compile_error(format!(
                            "function `{name}` does not return a value"
                        )));
                    };

                    if let Some(expected) = expected {
                        expect_type(return_type, expected)
                    } else {
                        Ok(return_type.clone())
                    }
                }
            },
            Expression::StructLiteral(literal) => {
                let ty = Type::Struct(literal.name.clone());
                if let Some(expected) = expected {
                    expect_type(&ty, expected)
                } else {
                    Ok(ty)
                }
            }
            Expression::FieldAccess(access) => {
                let object_ty = self.expression_type(&access.object, None)?;
                let struct_name = match object_ty {
                    Type::Struct(name) => name,
                    Type::Pointer(inner) => match *inner {
                        Type::Struct(name) => name,
                        ty => {
                            return Err(compile_error(format!(
                                "cannot access field `{}` on `&{}`",
                                access.field,
                                ty.name()
                            )));
                        }
                    },
                    ty => {
                        return Err(compile_error(format!(
                            "cannot access field `{}` on `{}`",
                            access.field,
                            ty.name()
                        )));
                    }
                };
                let ty = self
                    .struct_layout(&struct_name)?
                    .fields
                    .get(&access.field)
                    .map(|field| field.ty.clone())
                    .ok_or_else(|| {
                        compile_error(format!(
                            "unknown field `{}` in `{struct_name}`",
                            access.field
                        ))
                    })?;

                if let Some(expected) = expected {
                    expect_type(&ty, expected)
                } else {
                    Ok(ty)
                }
            }
            Expression::AddressOf(expression) => {
                let inner = if let Some(Type::Pointer(inner)) = expected {
                    inner.as_ref().clone()
                } else {
                    self.expression_type(expression, None)?
                };
                Ok(Type::Pointer(Box::new(inner)))
            }
            Expression::Dereference(expression) => {
                let pointer_ty = self.expression_type(expression, None)?;
                let Type::Pointer(inner) = pointer_ty else {
                    return Err(compile_error(format!(
                        "cannot dereference `{}`",
                        pointer_ty.name()
                    )));
                };

                if let Some(expected) = expected {
                    expect_type(&inner, expected)
                } else {
                    Ok(*inner)
                }
            }
            Expression::BitNot(expression) => {
                let operand_ty = if let Some(expected) = expected {
                    expected.clone()
                } else {
                    self.expression_type(expression, None)?
                };

                if operand_ty.is_bool() || operand_ty.is_integer() {
                    Ok(operand_ty)
                } else {
                    Err(compile_error(format!(
                        "operator `!` cannot be used with `{}`",
                        operand_ty.name()
                    )))
                }
            }
            Expression::Binary(binary) => match binary.operator {
                BinaryOperator::BoolAnd
                | BinaryOperator::BoolOr
                | BinaryOperator::Equal
                | BinaryOperator::NotEqual => {
                    if let Some(expected) = expected {
                        expect_type(&Type::Bool, expected)
                    } else {
                        Ok(Type::Bool)
                    }
                }
                _ => self.binary_operand_type(binary, expected),
            },
        }
    }

    fn binary_operand_type(
        &self,
        binary: &BinaryExpression,
        expected: Option<&Type>,
    ) -> Result<Type, CompileError> {
        if let Some(expected) = expected {
            return Ok(expected.clone());
        }

        if is_untyped_literal(&binary.left) && !is_untyped_literal(&binary.right) {
            self.expression_type(&binary.right, None)
        } else {
            self.expression_type(&binary.left, None)
        }
    }

    fn compile_float_literal(&mut self, value: &str, ty: &Type) -> Result<(), CompileError> {
        let parsed = value
            .parse::<f64>()
            .map_err(|_| compile_error(format!("float literal `{value}` is invalid")))?;
        let bits = match ty {
            Type::F16 | Type::F32 => (parsed as f32).to_bits() as u64,
            Type::F64 => parsed.to_bits(),
            _ => {
                return Err(compile_error(format!(
                    "cannot assign float literal to `{}`",
                    ty.name()
                )));
            }
        };

        emit_mov_u64(&mut self.body, "x0", bits);
        Ok(())
    }

    fn compile_integer_literal(&mut self, value: u128, ty: &Type) {
        emit_mov_u64(&mut self.body, "x0", value as u64);
        if is_128_integer(ty) {
            emit_mov_u64(&mut self.body, "x1", (value >> 64) as u64);
        } else {
            self.normalize_x0(ty);
        }
    }

    fn emit_printf_with_x0(&mut self, format_label: &str) {
        self.body.push_str("    mov x1, x0\n");
        self.body.push_str("    sub sp, sp, #16\n");
        self.body.push_str("    str x1, [sp]\n");
        emit_load_label_address(&mut self.body, "x0", format_label);
        self.body.push_str("    bl _printf\n");
        self.body.push_str("    add sp, sp, #16\n");
    }

    fn local(&self, name: &str) -> Result<Local, CompileError> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name))
            .cloned()
            .ok_or_else(|| compile_error(format!("variable `{name}` not found")))
    }

    fn insert_local(&mut self, name: String, local: Local) -> Result<(), CompileError> {
        self.scopes
            .last_mut()
            .expect("there is always a scope")
            .insert(name, local);
        Ok(())
    }

    fn allocate_local(&mut self, ty: Type, storage: LocalStorage) -> Result<Local, CompileError> {
        let size = if storage == LocalStorage::Indirect {
            8
        } else {
            self.type_size(&ty)?
        };
        self.next_offset = align_to(self.next_offset, 8) + align_to(size, 8);

        Ok(Local {
            ty,
            offset: self.next_offset,
            storage,
        })
    }

    fn allocate_temp_slot(&mut self) -> Result<usize, CompileError> {
        let local = self.allocate_local(Type::U64, LocalStorage::Direct)?;
        Ok(local.offset)
    }

    fn local_address(&mut self, local: &Local, register: &str) -> Result<(), CompileError> {
        match local.storage {
            LocalStorage::Direct => {
                emit_sub_imm(&mut self.body, register, "x29", local.offset);
                Ok(())
            }
            LocalStorage::Indirect => self.load_frame_to_register(local.offset, register),
        }
    }

    fn local_field_address(
        &mut self,
        local: &Local,
        field_offset: usize,
        register: &str,
    ) -> Result<(), CompileError> {
        if local.storage != LocalStorage::Direct {
            return Err(compile_error(
                "internal error: indirect local field address",
            ));
        }

        emit_sub_imm(&mut self.body, register, "x29", local.offset - field_offset);
        Ok(())
    }

    fn store_register_to_frame(
        &mut self,
        register: &str,
        offset: usize,
    ) -> Result<(), CompileError> {
        emit_sub_imm(&mut self.body, "x9", "x29", offset);
        writeln!(self.body, "    str {register}, [x9]").unwrap();
        Ok(())
    }

    fn store_register_pair_to_frame(
        &mut self,
        low_register: &str,
        high_register: &str,
        offset: usize,
    ) -> Result<(), CompileError> {
        emit_sub_imm(&mut self.body, "x9", "x29", offset);
        writeln!(self.body, "    str {low_register}, [x9]").unwrap();
        writeln!(self.body, "    str {high_register}, [x9, #8]").unwrap();
        Ok(())
    }

    fn store_value_to_local(&mut self, ty: &Type, local: &Local) -> Result<(), CompileError> {
        if local.storage != LocalStorage::Direct {
            return Err(compile_error(
                "internal error: cannot store direct value to indirect local",
            ));
        }

        emit_sub_imm(&mut self.body, "x9", "x29", local.offset);
        self.store_value_to_address(ty, "x9")
    }

    fn store_value_to_address(
        &mut self,
        ty: &Type,
        address_register: &str,
    ) -> Result<(), CompileError> {
        writeln!(self.body, "    str x0, [{address_register}]").unwrap();
        if is_128_integer(ty) {
            writeln!(self.body, "    str x1, [{address_register}, #8]").unwrap();
        }
        Ok(())
    }

    fn load_local_to_x0(&mut self, local: &Local) -> Result<(), CompileError> {
        if local.storage != LocalStorage::Direct {
            return Err(compile_error(
                "internal error: cannot load indirect local as value",
            ));
        }

        self.load_frame_value_to_x0_x1(local.offset, &local.ty)?;
        Ok(())
    }

    fn load_frame_value_to_x0_x1(&mut self, offset: usize, ty: &Type) -> Result<(), CompileError> {
        emit_sub_imm(&mut self.body, "x9", "x29", offset);
        self.body.push_str("    ldr x0, [x9]\n");
        if is_128_integer(ty) {
            self.body.push_str("    ldr x1, [x9, #8]\n");
        } else {
            self.normalize_x0(ty);
        }
        Ok(())
    }

    fn load_frame_to_register(
        &mut self,
        offset: usize,
        register: &str,
    ) -> Result<(), CompileError> {
        emit_sub_imm(&mut self.body, "x9", "x29", offset);
        writeln!(self.body, "    ldr {register}, [x9]").unwrap();
        Ok(())
    }

    fn load_address_to_x0(&mut self, ty: &Type) -> Result<(), CompileError> {
        if is_128_integer(ty) {
            self.body.push_str("    ldr x1, [x0, #8]\n");
            self.body.push_str("    ldr x0, [x0]\n");
        } else {
            self.body.push_str("    ldr x0, [x0]\n");
            self.normalize_x0(ty);
        }
        Ok(())
    }

    fn copy_value_to_address(
        &mut self,
        ty: &Type,
        source_register: &str,
        destination_register: &str,
    ) -> Result<(), CompileError> {
        let size = self.type_size(ty)?;

        for offset in (0..size).step_by(8) {
            writeln!(self.body, "    ldr x10, [{source_register}, #{}]", offset).unwrap();
            writeln!(
                self.body,
                "    str x10, [{destination_register}, #{}]",
                offset
            )
            .unwrap();
        }

        Ok(())
    }

    fn push_value(&mut self, ty: &Type) {
        self.body.push_str("    sub sp, sp, #16\n");
        self.body.push_str("    str x0, [sp]\n");
        if is_128_integer(ty) {
            self.body.push_str("    str x1, [sp, #8]\n");
        }
    }

    fn pop_value_to_x0_x1(&mut self, ty: &Type) {
        self.body.push_str("    ldr x0, [sp]\n");
        if is_128_integer(ty) {
            self.body.push_str("    ldr x1, [sp, #8]\n");
        }
        self.body.push_str("    add sp, sp, #16\n");
    }

    fn normalize_x0(&mut self, ty: &Type) {
        match ty {
            Type::I8 => self.body.push_str("    sxtb x0, w0\n"),
            Type::I16 => self.body.push_str("    sxth x0, w0\n"),
            Type::I32 => self.body.push_str("    sxtw x0, w0\n"),
            Type::U8 | Type::Bool => self.body.push_str("    and x0, x0, #0xff\n"),
            Type::U16 => self.body.push_str("    and x0, x0, #0xffff\n"),
            Type::U32 => self.body.push_str("    mov w0, w0\n"),
            _ => {}
        }
    }

    fn type_size(&self, ty: &Type) -> Result<usize, CompileError> {
        match ty {
            Type::Struct(name) => Ok(self.struct_layout(name)?.size),
            Type::I128 | Type::U128 => Ok(16),
            _ => Ok(8),
        }
    }

    fn struct_layout(&self, name: &str) -> Result<&StructLayout, CompileError> {
        self.layouts
            .get(name)
            .ok_or_else(|| compile_error(format!("unknown type `{name}`")))
    }

    fn make_label(&mut self, prefix: &str) -> String {
        let label = format!("L_mould_{prefix}_{}", *self.next_label);
        *self.next_label += 1;
        label
    }
}

fn build_struct_layouts(
    checked: &CheckedProgram,
) -> Result<HashMap<String, StructLayout>, CompileError> {
    let mut layouts = HashMap::new();
    let mut visiting = Vec::new();

    for name in checked.structs.keys() {
        build_struct_layout(name, checked, &mut layouts, &mut visiting)?;
    }

    Ok(layouts)
}

fn build_struct_layout(
    name: &str,
    checked: &CheckedProgram,
    layouts: &mut HashMap<String, StructLayout>,
    visiting: &mut Vec<String>,
) -> Result<(), CompileError> {
    if layouts.contains_key(name) {
        return Ok(());
    }

    if visiting.iter().any(|visited| visited == name) {
        return Err(compile_error(format!(
            "recursive struct `{name}` is not supported"
        )));
    }

    let structure = checked
        .structs
        .get(name)
        .ok_or_else(|| compile_error(format!("unknown type `{name}`")))?;

    visiting.push(name.to_string());

    let mut fields = HashMap::new();
    let mut offset = 0;

    for field in &structure.fields {
        let size = match &field.ty {
            Type::Struct(name) => {
                build_struct_layout(name, checked, layouts, visiting)?;
                layouts
                    .get(name)
                    .expect("struct layout was just built")
                    .size
            }
            Type::I128 | Type::U128 => 16,
            _ => 8,
        };
        fields.insert(
            field.name.clone(),
            FieldLayout {
                offset,
                ty: field.ty.clone(),
            },
        );
        offset += align_to(size, 8);
    }

    visiting.pop();
    layouts.insert(
        name.to_string(),
        StructLayout {
            size: align_to(offset, 8),
            fields,
        },
    );
    Ok(())
}

fn ensure_supported_type(ty: &Type) -> Result<(), CompileError> {
    match ty {
        Type::Pointer(inner) => ensure_supported_type(inner),
        Type::Struct(_) => Ok(()),
        _ => Ok(()),
    }
}

fn is_128_integer(ty: &Type) -> bool {
    matches!(ty, Type::I128 | Type::U128)
}

fn value_register_count(ty: &Type) -> usize {
    if is_128_integer(ty) { 2 } else { 1 }
}

fn expect_type(actual: &Type, expected: &Type) -> Result<Type, CompileError> {
    if actual == expected {
        Ok(actual.clone())
    } else {
        Err(compile_error(format!(
            "cannot use `{}` value as `{}`",
            actual.name(),
            expected.name()
        )))
    }
}

fn is_untyped_literal(expression: &Expression) -> bool {
    matches!(expression, Expression::Integer(_) | Expression::Float(_))
}

fn emit_load_label_address(text: &mut String, register: &str, label: &str) {
    writeln!(text, "    adrp {register}, {label}@PAGE").unwrap();
    writeln!(text, "    add {register}, {register}, {label}@PAGEOFF").unwrap();
}

fn emit_mov_u64(text: &mut String, register: &str, value: u64) {
    writeln!(text, "    movz {register}, #{}", value & 0xffff).unwrap();

    for shift in [16, 32, 48] {
        let part = (value >> shift) & 0xffff;
        if part != 0 {
            writeln!(text, "    movk {register}, #{part}, lsl #{shift}").unwrap();
        }
    }
}

fn emit_sub_imm(text: &mut String, destination: &str, source: &str, value: usize) {
    if value == 0 {
        if destination != source {
            writeln!(text, "    mov {destination}, {source}").unwrap();
        }
        return;
    }

    if value <= 4095 {
        writeln!(text, "    sub {destination}, {source}, #{value}").unwrap();
    } else {
        emit_mov_u64(text, "x16", value as u64);
        writeln!(text, "    sub {destination}, {source}, x16").unwrap();
    }
}

fn emit_add_imm(text: &mut String, destination: &str, source: &str, value: usize) {
    if value == 0 {
        if destination != source {
            writeln!(text, "    mov {destination}, {source}").unwrap();
        }
        return;
    }

    if value <= 4095 {
        writeln!(text, "    add {destination}, {source}, #{value}").unwrap();
    } else {
        emit_mov_u64(text, "x16", value as u64);
        writeln!(text, "    add {destination}, {source}, x16").unwrap();
    }
}

fn function_label(name: &str) -> String {
    if name == "main" {
        "_main".to_string()
    } else {
        format!("_mould_{name}")
    }
}

fn align_to(value: usize, alignment: usize) -> usize {
    value.div_ceil(alignment) * alignment
}

fn temporary_assembly_path() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();

    std::env::temp_dir().join(format!("mould-{}-{nanos}.s", std::process::id()))
}

fn compile_error(message: impl Into<String>) -> CompileError {
    CompileError {
        message: message.into(),
    }
}

const U128_PRINT_HELPERS: &str = r#"
.globl _mould_print_i128
.p2align 2
_mould_print_i128:
    stp x29, x30, [sp, #-16]!
    mov x29, sp
    tbz x1, #63, L_mould_print_i128_positive
    mvn x0, x0
    mvn x1, x1
    adds x0, x0, #1
    adc x1, x1, xzr
    sub sp, sp, #16
    stp x0, x1, [sp]
    mov x0, #45
    bl _putchar
    ldp x0, x1, [sp]
    add sp, sp, #16
L_mould_print_i128_positive:
    bl _mould_print_u128
    ldp x29, x30, [sp], #16
    ret

.globl _mould_print_u128
.p2align 2
_mould_print_u128:
    stp x29, x30, [sp, #-16]!
    mov x29, sp
    stp x19, x20, [sp, #-16]!
    sub sp, sp, #64
    add x19, sp, #63
    strb wzr, [x19]
    cmp x0, #0
    ccmp x1, #0, #0, eq
    b.ne L_mould_print_u128_loop
    sub x19, x19, #1
    mov w6, #48
    strb w6, [x19]
    b L_mould_print_u128_emit
L_mould_print_u128_loop:
    bl L_mould_udivmod10_u128
    sub x19, x19, #1
    add w6, w2, #48
    strb w6, [x19]
    cmp x0, #0
    ccmp x1, #0, #0, eq
    b.ne L_mould_print_u128_loop
L_mould_print_u128_emit:
    mov x0, x19
    bl _puts
    add sp, sp, #64
    ldp x19, x20, [sp], #16
    ldp x29, x30, [sp], #16
    ret

.p2align 2
L_mould_udivmod10_u128:
    mov x6, x0
    mov x7, x1
    mov x0, #0
    mov x1, #0
    mov x2, #0
    mov x5, #128
L_mould_udivmod10_u128_loop:
    lsr x8, x7, #63
    lsl x2, x2, #1
    orr x2, x2, x8
    cmp x2, #10
    b.lo L_mould_udivmod10_u128_no_sub
    sub x2, x2, #10
    mov x8, #1
    b L_mould_udivmod10_u128_have_bit
L_mould_udivmod10_u128_no_sub:
    mov x8, #0
L_mould_udivmod10_u128_have_bit:
    adds x0, x0, x0
    adcs x1, x1, x1
    orr x0, x0, x8
    adds x6, x6, x6
    adc x7, x7, x7
    subs x5, x5, #1
    b.ne L_mould_udivmod10_u128_loop
    ret

.p2align 2
L_mould_udivmod_u128:
    cmp x2, #0
    ccmp x3, #0, #0, eq
    b.ne L_mould_udivmod_u128_nonzero
    mov x0, #0
    mov x1, #0
    mov x2, #0
    mov x3, #0
    ret
L_mould_udivmod_u128_nonzero:
    mov x6, x0
    mov x7, x1
    mov x0, #0
    mov x1, #0
    mov x4, #0
    mov x5, #0
    mov x9, #128
L_mould_udivmod_u128_loop:
    lsr x8, x7, #63
    adds x4, x4, x4
    adcs x5, x5, x5
    orr x4, x4, x8
    cmp x5, x3
    b.hi L_mould_udivmod_u128_sub
    b.lo L_mould_udivmod_u128_no_sub
    cmp x4, x2
    b.lo L_mould_udivmod_u128_no_sub
L_mould_udivmod_u128_sub:
    subs x4, x4, x2
    sbc x5, x5, x3
    mov x8, #1
    b L_mould_udivmod_u128_have_bit
L_mould_udivmod_u128_no_sub:
    mov x8, #0
L_mould_udivmod_u128_have_bit:
    adds x0, x0, x0
    adcs x1, x1, x1
    orr x0, x0, x8
    adds x6, x6, x6
    adc x7, x7, x7
    subs x9, x9, #1
    b.ne L_mould_udivmod_u128_loop
    mov x2, x4
    mov x3, x5
    ret

"#;

#[cfg(test)]
mod tests {
    use frontend::parse_source;

    use super::{CompileError, compile_program_to_assembly};

    #[test]
    fn emits_println_number() {
        let assembly = compile("fn main() { println(1) }");

        assert!(assembly.contains("bl _printf"));
        assert!(assembly.contains("L_mould_fmt_signed"));
    }

    #[test]
    fn emits_function_with_params_and_return() {
        let assembly = compile(
            "fn sample(a: i32, b: bool) -> i32 { return a } fn main() { println(sample(7, true)) }",
        );

        assert!(assembly.contains("_mould_sample:"));
        assert!(assembly.contains("bl _mould_sample"));
    }

    #[test]
    fn emits_struct_field_access() {
        let assembly = compile(
            "struct Point { x: i32, y: bool } fn main() { let p: Point = Point { x: 7, y: true } println(p.x) }",
        );

        assert!(assembly.contains("ldr x0, [x0]"));
    }

    #[test]
    fn emits_allocated_value() {
        let assembly = compile("fn main() { let p: &i32 = alloc(7) println(*p) dealloc(p) }");

        assert!(assembly.contains("bl _malloc"));
        assert!(assembly.contains("bl _free"));
    }

    #[test]
    fn emits_math_instructions() {
        let assembly = compile("fn main() { let n: i32 = 1 + 2 * 3 println(n) }");

        assert!(assembly.contains("mul x0, x0, x1"));
        assert!(assembly.contains("add x0, x0, x1"));
    }

    #[test]
    fn emits_bitwise_instructions() {
        let assembly = compile("fn main() { let n: u8 = 10 & 12 println(n) }");

        assert!(assembly.contains("and x0, x0, x1"));
    }

    #[test]
    fn emits_if_branching() {
        let assembly =
            compile("fn main() { if true && !false { println(1) } else { println(2) } }");

        assert!(assembly.contains("b.eq L_mould_if_else"));
    }

    #[test]
    fn emits_loop_branching() {
        let assembly = compile("fn main() { loop { println(1) break } }");

        assert!(assembly.contains("L_mould_loop_start"));
        assert!(assembly.contains("L_mould_loop_end"));
    }

    #[test]
    fn rejects_missing_main() {
        let error = compile_error("fn helper() {}");

        assert!(error.message.contains("function `main` not found"));
    }

    #[test]
    fn rejects_unknown_struct() {
        let error = compile_error("fn main() { let p: Point = Point { x: 1 } }");

        assert!(error.message.contains("unknown type `Point`"));
    }

    #[test]
    fn rejects_missing_field() {
        let error = compile_error(
            "struct Point { x: i32, y: bool } fn main() { let p: Point = Point { x: 1 } }",
        );

        assert!(error.message.contains("missing field `y`"));
    }

    #[test]
    fn emits_i128_storage_and_printing() {
        let assembly = compile("fn main() { let value: i128 = 1 println(value) }");

        assert!(assembly.contains("str x1, [x9, #8]"));
        assert!(assembly.contains("bl _mould_print_i128"));
    }

    #[test]
    fn emits_i128_division_helper() {
        let assembly = compile("fn main() { let value: u128 = 8 / 2 println(value) }");

        assert!(assembly.contains("bl L_mould_udivmod_u128"));
    }

    fn compile(source: &str) -> String {
        let program = parse_source(source).unwrap();
        compile_program_to_assembly(&program).unwrap()
    }

    fn compile_error(source: &str) -> CompileError {
        let program = parse_source(source).unwrap();
        compile_program_to_assembly(&program).unwrap_err()
    }
}
