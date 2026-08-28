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
    let c_source = compile_program_to_c(program)?;
    let c_path = temporary_c_path();

    fs::write(&c_path, c_source).map_err(|error| CompileError {
        message: format!("failed to write `{}`: {error}", c_path.display()),
    })?;

    let output = Command::new("cc")
        .arg(&c_path)
        .arg("-o")
        .arg(output_path)
        .output()
        .map_err(|error| CompileError {
            message: format!("failed to run `cc`: {error}"),
        })?;

    let _ = fs::remove_file(&c_path);

    if output.status.success() {
        return Ok(());
    }

    Err(CompileError {
        message: format!(
            "native compiler failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    })
}

pub fn compile_program_to_c(program: &Program) -> Result<String, CompileError> {
    let known_functions = collect_functions(program)?;

    if !known_functions.contains("main") {
        return Err(CompileError {
            message: "function `main` not found".to_string(),
        });
    }

    let mut output = String::new();
    output.push_str("#include <stdint.h>\n");
    output.push_str("#include <stdio.h>\n\n");

    for function in &program.functions {
        writeln!(
            output,
            "static void {}(void);",
            c_function_name(&function.name)
        )
        .unwrap();
    }

    output.push('\n');

    for function in &program.functions {
        emit_function(&mut output, function, &known_functions)?;
        output.push('\n');
    }

    output.push_str("int main(void) {\n");
    output.push_str("    mould_fn_main();\n");
    output.push_str("    return 0;\n");
    output.push_str("}\n");

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
    writeln!(
        output,
        "static void {}(void) {{",
        c_function_name(&function.name)
    )
    .unwrap();

    let mut compiler = FunctionCompiler::new(known_functions);

    for statement in &function.body.statements {
        compiler.emit_statement(output, statement)?;
    }

    output.push_str("}\n");
    Ok(())
}

struct FunctionCompiler<'program> {
    known_functions: &'program HashSet<String>,
    variables: HashMap<String, String>,
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
        let value = self.emit_expression(&statement.value)?;
        let c_name = format!(
            "{}_{}",
            c_variable_name(&statement.name),
            self.next_variable
        );

        self.next_variable += 1;
        writeln!(output, "    int32_t {c_name} = {value};").unwrap();
        self.variables.insert(statement.name.clone(), c_name);

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

        writeln!(output, "    {}();", c_function_name(&statement.name)).unwrap();
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

        let value = self.emit_expression(&statement.arguments[0])?;
        writeln!(output, "    printf(\"%d\\n\", {value});").unwrap();

        Ok(())
    }

    fn emit_expression(&self, expression: &Expression) -> Result<String, CompileError> {
        match expression {
            Expression::Integer(value) => Ok(value.to_string()),
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
}

fn c_function_name(name: &str) -> String {
    c_name("mould_fn_", name)
}

fn c_variable_name(name: &str) -> String {
    c_name("mould_var_", name)
}

fn c_name(prefix: &str, name: &str) -> String {
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

fn temporary_c_path() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();

    std::env::temp_dir().join(format!("mould-{}-{nanos}.c", std::process::id()))
}

#[cfg(test)]
mod tests {
    use frontend::parse_source;

    use super::{CompileError, compile_program_to_c};

    #[test]
    fn emits_println_number() {
        let c_source = compile("fn main() { println(1) }");

        assert!(c_source.contains("printf(\"%d\\n\", 1);"));
    }

    #[test]
    fn emits_variable_and_println() {
        let c_source = compile("fn main() { let a: i32 = 1 println(a) }");

        assert!(c_source.contains("int32_t mould_var_a_0 = 1;"));
        assert!(c_source.contains("printf(\"%d\\n\", mould_var_a_0);"));
    }

    #[test]
    fn emits_user_function_call() {
        let c_source = compile("fn helper() { println(1) } fn main() { helper() }");

        assert!(c_source.contains("static void mould_fn_helper(void);"));
        assert!(c_source.contains("mould_fn_helper();"));
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
        compile_program_to_c(&program).unwrap()
    }

    fn compile_error(source: &str) -> CompileError {
        let program = parse_source(source).unwrap();
        compile_program_to_c(&program).unwrap_err()
    }
}
