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
};

const PRINTLN_I32_FORMAT_LABEL: &str = "L_mould_println_i32_format";

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

    let mut output = String::new();
    output.push_str(".build_version macos, 11, 0\n");
    output.push_str(".section __TEXT,__cstring,cstring_literals\n");
    writeln!(output, "{PRINTLN_I32_FORMAT_LABEL}:").unwrap();
    output.push_str("    .asciz \"%d\\n\"\n\n");
    output.push_str(".section __TEXT,__text,regular,pure_instructions\n\n");

    for function in &program.functions {
        emit_function(&mut output, function, &known_functions)?;
        output.push('\n');
    }

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
    output: &mut String,
    function: &Function,
    known_functions: &HashSet<String>,
) -> Result<(), CompileError> {
    let stack_size = align_to_16(count_local_variables(function) * 4);

    output.push_str(".p2align 2\n");
    writeln!(output, "{}:", asm_function_name(&function.name)).unwrap();
    output.push_str("    stp x29, x30, [sp, #-16]!\n");
    output.push_str("    mov x29, sp\n");

    if stack_size > 0 {
        writeln!(output, "    sub sp, sp, #{stack_size}").unwrap();
    }

    let mut compiler = FunctionCompiler::new(known_functions);

    for statement in &function.body.statements {
        compiler.emit_statement(output, statement)?;
    }

    if stack_size > 0 {
        writeln!(output, "    add sp, sp, #{stack_size}").unwrap();
    }

    output.push_str("    ldp x29, x30, [sp], #16\n");
    output.push_str("    ret\n");
    Ok(())
}

fn count_local_variables(function: &Function) -> usize {
    function
        .body
        .statements
        .iter()
        .filter(|statement| matches!(statement, Let(_)))
        .count()
}

struct FunctionCompiler<'program> {
    known_functions: &'program HashSet<String>,
    variables: HashMap<String, usize>,
    next_variable: usize,
}

impl<'program> FunctionCompiler<'program> {
    fn new(known_functions: &'program HashSet<String>) -> Self {
        Self {
            known_functions,
            variables: HashMap::new(),
            next_variable: 0,
        }
    }

    fn emit_statement(
        &mut self,
        output: &mut String,
        statement: &Statement,
    ) -> Result<(), CompileError> {
        match statement {
            Let(statement) => self.emit_let_statement(output, statement),
            Call(statement) => self.emit_call_statement(output, statement),
        }
    }

    fn emit_let_statement(
        &mut self,
        output: &mut String,
        statement: &LetStatement,
    ) -> Result<(), CompileError> {
        self.emit_expression(output, &statement.value)?;

        let offset = (self.next_variable + 1) * 4;
        self.next_variable += 1;
        self.emit_stack_address(output, offset)?;
        output.push_str("    str w8, [x9]\n");
        self.variables.insert(statement.name.clone(), offset);

        Ok(())
    }

    fn emit_call_statement(
        &mut self,
        output: &mut String,
        statement: &CallStatement,
    ) -> Result<(), CompileError> {
        if statement.name == "println" {
            return self.emit_println(output, statement);
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

        writeln!(output, "    bl {}", asm_function_name(&statement.name)).unwrap();
        Ok(())
    }

    fn emit_println(
        &mut self,
        output: &mut String,
        statement: &CallStatement,
    ) -> Result<(), CompileError> {
        if statement.arguments.len() != 1 {
            return Err(CompileError {
                message: "function `println` expects 1 argument".to_string(),
            });
        }

        self.emit_expression(output, &statement.arguments[0])?;
        output.push_str("    sub sp, sp, #16\n");
        output.push_str("    sxtw x8, w8\n");
        output.push_str("    str x8, [sp]\n");
        writeln!(output, "    adrp x0, {PRINTLN_I32_FORMAT_LABEL}@PAGE").unwrap();
        writeln!(output, "    add x0, x0, {PRINTLN_I32_FORMAT_LABEL}@PAGEOFF").unwrap();
        output.push_str("    bl _printf\n");
        output.push_str("    add sp, sp, #16\n");

        Ok(())
    }

    fn emit_expression(
        &self,
        output: &mut String,
        expression: &Expression,
    ) -> Result<(), CompileError> {
        match expression {
            Expression::Integer(value) => {
                emit_load_i32(output, *value);
                Ok(())
            }
            Expression::Variable(name) => {
                let offset = self.variables.get(name).ok_or_else(|| CompileError {
                    message: format!("variable `{name}` not found"),
                })?;

                self.emit_stack_address(output, *offset)?;
                output.push_str("    ldr w8, [x9]\n");
                Ok(())
            }
        }
    }

    fn emit_stack_address(&self, output: &mut String, offset: usize) -> Result<(), CompileError> {
        if offset > 4095 {
            return Err(CompileError {
                message: "function has too many local variables".to_string(),
            });
        }

        writeln!(output, "    sub x9, x29, #{offset}").unwrap();
        Ok(())
    }
}

fn emit_load_i32(output: &mut String, value: i32) {
    let bits = value as u32;
    let low = bits & 0xffff;
    let high = bits >> 16;

    writeln!(output, "    movz w8, #{low}").unwrap();

    if high != 0 {
        writeln!(output, "    movk w8, #{high}, lsl #16").unwrap();
    }
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

fn align_to_16(value: usize) -> usize {
    (value + 15) & !15
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

        assert!(assembly.contains("movz w8, #1"));
        assert!(assembly.contains("bl _printf"));
    }

    #[test]
    fn emits_variable_and_println() {
        let assembly = compile("fn main() { let a: i32 = 1 println(a) }");

        assert!(assembly.contains("sub sp, sp, #16"));
        assert!(assembly.contains("str w8, [x9]"));
        assert!(assembly.contains("ldr w8, [x9]"));
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

    fn compile(source: &str) -> String {
        let program = parse_source(source).unwrap();
        compile_program_to_assembly(&program).unwrap()
    }

    fn compile_error(source: &str) -> CompileError {
        let program = parse_source(source).unwrap();
        compile_program_to_assembly(&program).unwrap_err()
    }
}
