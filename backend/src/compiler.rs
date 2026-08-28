use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use frontend::Program;

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
    let output = frontend::run_main(program).map_err(|error| CompileError {
        message: error.message,
    })?;

    Ok(AssemblyEmitter::new().finish(&output))
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

    fn finish(mut self, output: &str) -> String {
        for line in output.lines() {
            self.emit_puts(line);
        }

        let mut assembly = String::new();
        assembly.push_str(".build_version macos, 11, 0\n");
        assembly.push_str(".section __TEXT,__cstring,cstring_literals\n");
        assembly.push_str(&self.cstrings);
        assembly.push('\n');
        assembly.push_str(".section __TEXT,__text,regular,pure_instructions\n\n");
        assembly.push_str(".globl _main\n");
        assembly.push_str(".p2align 2\n");
        assembly.push_str("_main:\n");
        assembly.push_str("    stp x29, x30, [sp, #-16]!\n");
        assembly.push_str("    mov x29, sp\n");
        assembly.push_str(&self.text);
        assembly.push_str("    mov w0, #0\n");
        assembly.push_str("    ldp x29, x30, [sp], #16\n");
        assembly.push_str("    ret\n\n");
        assembly.push_str(".subsections_via_symbols\n");

        assembly
    }

    fn emit_puts(&mut self, value: &str) {
        let label = self.add_cstring(value);

        writeln!(self.text, "    adrp x0, {label}@PAGE").unwrap();
        writeln!(self.text, "    add x0, x0, {label}@PAGEOFF").unwrap();
        self.text.push_str("    bl _puts\n");
    }

    fn add_cstring(&mut self, value: &str) -> String {
        let label = format!("L_mould_string_{}", self.next_string);
        self.next_string += 1;

        writeln!(self.cstrings, "{label}:").unwrap();
        writeln!(self.cstrings, "    .asciz \"{}\"", escape_cstring(value)).unwrap();

        label
    }
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
    fn emits_function_with_params_and_return() {
        let assembly = compile(
            "fn sample(a: i32, b: bool) -> i32 { return a } fn main() { println(sample(7, true)) }",
        );

        assert!(assembly.contains(".asciz \"7\""));
    }

    #[test]
    fn emits_struct_field_access() {
        let assembly = compile(
            "struct Point { x: i32, y: bool } fn main() { let p: Point = Point { x: 7, y: true } println(p.x) }",
        );

        assert!(assembly.contains(".asciz \"7\""));
    }

    #[test]
    fn emits_struct_value() {
        let assembly = compile(
            "struct Point { x: i32, y: bool } fn main() { let p: Point = Point { x: 7, y: true } println(p) }",
        );

        assert!(assembly.contains(".asciz \"Point { x: 7, y: true }\""));
    }

    #[test]
    fn emits_allocated_value() {
        let assembly = compile("fn main() { let p: &i32 = alloc(7) println(*p) dealloc(p) }");

        assert!(assembly.contains(".asciz \"7\""));
    }

    #[test]
    fn emits_math_result() {
        let assembly = compile("fn main() { let n: i32 = 1 + 2 * 3 println(n) }");

        assert!(assembly.contains(".asciz \"7\""));
    }

    #[test]
    fn emits_bitwise_result() {
        let assembly = compile("fn main() { let n: u8 = 10 & 12 println(n) }");

        assert!(assembly.contains(".asciz \"8\""));
    }

    #[test]
    fn emits_if_branch_result() {
        let assembly =
            compile("fn main() { if true && !false { println(1) } else { println(2) } }");

        assert!(assembly.contains(".asciz \"1\""));
    }

    #[test]
    fn rejects_use_after_dealloc() {
        let error = compile_error("fn main() { let p: &i32 = alloc(7) dealloc(p) println(*p) }");

        assert!(error.message.contains("freed pointer"));
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

    fn compile(source: &str) -> String {
        let program = parse_source(source).unwrap();
        compile_program_to_assembly(&program).unwrap()
    }

    fn compile_error(source: &str) -> CompileError {
        let program = parse_source(source).unwrap();
        compile_program_to_assembly(&program).unwrap_err()
    }
}
