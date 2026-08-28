use std::{
    collections::{HashMap, HashSet},
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use frontend::{
    CallStatement, Expression, Function, LetStatement, Program,
    Statement::{self, Call, Let},
    Type,
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

pub fn compile_source_to_executable(source: &str, output_path: &Path) -> Result<(), CompileError> {
    let program = frontend::parse_source(source).map_err(|error| CompileError {
        message: format!(
            "parse error at {}..{}: {}",
            error.span.start, error.span.end, error.message
        ),
    })?;

    compile_program_to_executable(&program, output_path)
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
    let known_functions = collect_functions(program)?;

    if !known_functions.contains("main") {
        return Err(CompileError {
            message: "function `main` not found".to_string(),
        });
    }

    let mut emitter = AssemblyEmitter::new();

    for function in &program.functions {
        emit_function(&mut emitter, function, &known_functions)?;
        emitter.text.push('\n');
    }

    let mut output = String::new();
    output.push_str(".build_version macos, 11, 0\n");
    output.push_str(".section __TEXT,__cstring,cstring_literals\n");
    output.push_str(&emitter.cstrings);
    output.push('\n');
    output.push_str(".section __TEXT,__text,regular,pure_instructions\n\n");
    output.push_str(&emitter.text);
    output.push_str(".globl _main\n");
    output.push_str(".p2align 2\n");
    output.push_str("_main:\n");
    output.push_str("    stp x29, x30, [sp, #-16]!\n");
    output.push_str("    mov x29, sp\n");
    output.push_str("    bl _mould_fn_main\n");
    output.push_str("    mov w0, #0\n");
    output.push_str("    ldp x29, x30, [sp], #16\n");
    output.push_str("    ret\n\n");
    output.push_str(".subsections_via_symbols\n");

    Ok(output)
}

fn collect_functions(program: &Program) -> Result<HashSet<String>, CompileError> {
    let mut functions = HashSet::new();

    for function in &program.functions {
        if function.name == "println" {
            return Err(CompileError {
                message: "cannot define builtin function `println`".to_string(),
            });
        }

        if !functions.insert(function.name.clone()) {
            return Err(CompileError {
                message: format!("function `{}` is already defined", function.name),
            });
        }
    }

    Ok(functions)
}

fn emit_function(
    emitter: &mut AssemblyEmitter,
    function: &Function,
    known_functions: &HashSet<String>,
) -> Result<(), CompileError> {
    emitter.text.push_str(".p2align 2\n");
    writeln!(emitter.text, "{}:", asm_function_name(&function.name)).unwrap();
    emitter.text.push_str("    stp x29, x30, [sp, #-16]!\n");
    emitter.text.push_str("    mov x29, sp\n");

    let mut compiler = FunctionCompiler::new(known_functions);

    for statement in &function.body.statements {
        compiler.emit_statement(emitter, statement)?;
    }

    emitter.text.push_str("    ldp x29, x30, [sp], #16\n");
    emitter.text.push_str("    ret\n");
    Ok(())
}

struct AssemblyEmitter {
    text: String,
    cstrings: String,
    next_string: usize,
}

impl AssemblyEmitter {
    fn new() -> Self {
        Self {
            text: String::new(),
            cstrings: String::new(),
            next_string: 0,
        }
    }

    fn add_cstring(&mut self, value: &str) -> String {
        let label = format!("L_mould_string_{}", self.next_string);
        self.next_string += 1;

        writeln!(self.cstrings, "{label}:").unwrap();
        writeln!(self.cstrings, "    .asciz \"{}\"", escape_cstring(value)).unwrap();

        label
    }

    fn emit_puts(&mut self, value: &str) {
        let label = self.add_cstring(value);

        writeln!(self.text, "    adrp x0, {label}@PAGE").unwrap();
        writeln!(self.text, "    add x0, x0, {label}@PAGEOFF").unwrap();
        self.text.push_str("    bl _puts\n");
    }
}

struct FunctionCompiler<'program> {
    known_functions: &'program HashSet<String>,
    variables: HashMap<String, Value>,
}

impl<'program> FunctionCompiler<'program> {
    fn new(known_functions: &'program HashSet<String>) -> Self {
        Self {
            known_functions,
            variables: HashMap::new(),
        }
    }

    fn emit_statement(
        &mut self,
        emitter: &mut AssemblyEmitter,
        statement: &Statement,
    ) -> Result<(), CompileError> {
        match statement {
            Let(statement) => self.emit_let_statement(statement),
            Call(statement) => self.emit_call_statement(emitter, statement),
        }
    }

    fn emit_let_statement(&mut self, statement: &LetStatement) -> Result<(), CompileError> {
        let value = self.evaluate_as(&statement.value, statement.ty)?;
        self.variables.insert(statement.name.clone(), value);
        Ok(())
    }

    fn emit_call_statement(
        &mut self,
        emitter: &mut AssemblyEmitter,
        statement: &CallStatement,
    ) -> Result<(), CompileError> {
        if statement.name == "println" {
            return self.emit_println(emitter, statement);
        }

        if !self.known_functions.contains(&statement.name) {
            return Err(CompileError {
                message: format!("unknown function `{}`", statement.name),
            });
        }

        if !statement.arguments.is_empty() {
            return Err(CompileError {
                message: format!("function `{}` expects 0 arguments", statement.name),
            });
        }

        writeln!(
            emitter.text,
            "    bl {}",
            asm_function_name(&statement.name)
        )
        .unwrap();
        Ok(())
    }

    fn emit_println(
        &mut self,
        emitter: &mut AssemblyEmitter,
        statement: &CallStatement,
    ) -> Result<(), CompileError> {
        if statement.arguments.len() != 1 {
            return Err(CompileError {
                message: "function `println` expects 1 argument".to_string(),
            });
        }

        let value = self.evaluate(&statement.arguments[0])?;
        emitter.emit_puts(&value.printable());

        Ok(())
    }

    fn evaluate(&self, expression: &Expression) -> Result<Value, CompileError> {
        match expression {
            Expression::Integer(value) => integer_value(*value, Type::I32),
            Expression::Float(value) => float_value(value, Type::F64),
            Expression::Bool(value) => Ok(Value::Bool(*value)),
            Expression::Variable(name) => {
                self.variables
                    .get(name)
                    .cloned()
                    .ok_or_else(|| CompileError {
                        message: format!("variable `{name}` not found"),
                    })
            }
        }
    }

    fn evaluate_as(&self, expression: &Expression, ty: Type) -> Result<Value, CompileError> {
        match expression {
            Expression::Integer(value) => integer_value(*value, ty),
            Expression::Float(value) => float_value(value, ty),
            Expression::Bool(value) => {
                if ty.is_bool() {
                    Ok(Value::Bool(*value))
                } else {
                    Err(CompileError {
                        message: format!("cannot assign bool literal to `{}`", ty.name()),
                    })
                }
            }
            Expression::Variable(name) => {
                let value = self
                    .variables
                    .get(name)
                    .cloned()
                    .ok_or_else(|| CompileError {
                        message: format!("variable `{name}` not found"),
                    })?;

                if value.ty() == ty {
                    Ok(value)
                } else {
                    Err(CompileError {
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

fn integer_value(value: u128, ty: Type) -> Result<Value, CompileError> {
    if !ty.is_integer() {
        return Err(CompileError {
            message: format!("cannot assign integer literal to `{}`", ty.name()),
        });
    }

    let max = ty.max_integer_value().expect("integer type has max value");

    if value > max {
        return Err(CompileError {
            message: format!("integer literal `{value}` does not fit in `{}`", ty.name()),
        });
    }

    Ok(Value::Integer { value, ty })
}

fn float_value(value: &str, ty: Type) -> Result<Value, CompileError> {
    if !ty.is_float() {
        return Err(CompileError {
            message: format!("cannot assign float literal to `{}`", ty.name()),
        });
    }

    let value = match ty {
        Type::F16 | Type::F32 => value.parse::<f32>().map(f64::from),
        Type::F64 => value.parse::<f64>(),
        _ => unreachable!("checked by is_float"),
    }
    .map_err(|_| CompileError {
        message: format!("float literal `{value}` is invalid"),
    })?;

    Ok(Value::Float { value, ty })
}

fn asm_function_name(name: &str) -> String {
    asm_name("_mould_fn_", name)
}

fn asm_name(prefix: &str, name: &str) -> String {
    let mut output = prefix.to_string();

    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            output.push(ch);
        } else {
            output.push('_');
        }
    }

    output
}

fn escape_cstring(value: &str) -> String {
    let mut escaped = String::new();

    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\t' => escaped.push_str("\\t"),
            _ => escaped.push(ch),
        }
    }

    escaped
}

fn temporary_assembly_path() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();

    std::env::temp_dir().join(format!("mould-{}-{nanos}.s", std::process::id()))
}

#[cfg(test)]
mod tests {
    use frontend::parse_source;

    use super::{CompileError, compile_program_to_assembly};

    #[test]
    fn emits_println_number() {
        let assembly = compile("fn main() { println(1) }");

        assert!(assembly.contains(".asciz \"1\""));
        assert!(assembly.contains("bl _puts"));
    }

    #[test]
    fn emits_variable_and_println() {
        let assembly = compile("fn main() { let a: i32 = 1 println(a) }");

        assert!(assembly.contains(".asciz \"1\""));
        assert!(assembly.contains("bl _puts"));
    }

    #[test]
    fn emits_bool_and_float() {
        let assembly =
            compile("fn main() { let a: bool = true println(a) let b: f64 = 1.5 println(b) }");

        assert!(assembly.contains(".asciz \"true\""));
        assert!(assembly.contains(".asciz \"1.5\""));
    }

    #[test]
    fn emits_u128() {
        let assembly = compile(
            "fn main() { let n: u128 = 340282366920938463463374607431768211455 println(n) }",
        );

        assert!(assembly.contains(".asciz \"340282366920938463463374607431768211455\""));
    }

    #[test]
    fn emits_user_function_call() {
        let assembly = compile("fn helper() { println(1) } fn main() { helper() }");

        assert!(assembly.contains("_mould_fn_helper:"));
        assert!(assembly.contains("bl _mould_fn_helper"));
    }

    #[test]
    fn rejects_missing_main() {
        let error = compile_error("fn helper() {}");

        assert!(error.message.contains("function `main` not found"));
    }

    #[test]
    fn rejects_unknown_function() {
        let error = compile_error("fn main() { nope() }");

        assert!(error.message.contains("unknown function `nope`"));
    }

    #[test]
    fn rejects_integer_out_of_range() {
        let error = compile_error("fn main() { let n: i8 = 128 }");

        assert!(error.message.contains("does not fit in `i8`"));
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
