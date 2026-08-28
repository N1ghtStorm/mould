use std::{
    collections::HashMap,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use frontend::{
    CallExpression, Expression, Function, Program,
    Statement::{self, Call, Let, Return},
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
    let mut compiler = ProgramCompiler::new(program)?;
    compiler.emit_main()?;
    Ok(compiler.finish())
}

struct ProgramCompiler<'program> {
    functions: HashMap<String, &'program Function>,
    emitter: AssemblyEmitter,
}

impl<'program> ProgramCompiler<'program> {
    fn new(program: &'program Program) -> Result<Self, CompileError> {
        let mut functions = HashMap::new();

        for function in &program.functions {
            if function.name == "println" {
                return Err(CompileError {
                    message: "cannot define builtin function `println`".to_string(),
                });
            }

            if functions.insert(function.name.clone(), function).is_some() {
                return Err(CompileError {
                    message: format!("function `{}` is already defined", function.name),
                });
            }
        }

        Ok(Self {
            functions,
            emitter: AssemblyEmitter::new(),
        })
    }

    fn emit_main(&mut self) -> Result<(), CompileError> {
        if !self.functions.contains_key("main") {
            return Err(CompileError {
                message: "function `main` not found".to_string(),
            });
        }

        self.call_function("main", &[], &HashMap::new())?;
        Ok(())
    }

    fn call_function(
        &mut self,
        name: &str,
        arguments: &[Expression],
        caller_variables: &HashMap<String, Value>,
    ) -> Result<Option<Value>, CompileError> {
        let function = self
            .functions
            .get(name)
            .copied()
            .cloned()
            .ok_or_else(|| CompileError {
                message: format!("unknown function `{name}`"),
            })?;

        if function.parameters.len() != arguments.len() {
            return Err(CompileError {
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

        let returned = self.emit_function_body(&function, &mut variables)?;

        match (function.return_type, returned) {
            (Some(_), Some(value)) => Ok(Some(value)),
            (Some(ty), None) => Err(CompileError {
                message: format!("function `{name}` must return `{}`", ty.name()),
            }),
            (None, Some(_)) => Err(CompileError {
                message: format!("function `{name}` cannot return a value"),
            }),
            (None, None) => Ok(None),
        }
    }

    fn emit_function_body(
        &mut self,
        function: &Function,
        variables: &mut HashMap<String, Value>,
    ) -> Result<Option<Value>, CompileError> {
        for statement in &function.body.statements {
            if let Some(value) = self.emit_statement(statement, function.return_type, variables)? {
                return Ok(Some(value));
            }
        }

        Ok(None)
    }

    fn emit_statement(
        &mut self,
        statement: &Statement,
        return_type: Option<Type>,
        variables: &mut HashMap<String, Value>,
    ) -> Result<Option<Value>, CompileError> {
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
                    return Err(CompileError {
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
    ) -> Result<Value, CompileError> {
        match expression {
            Expression::Integer(value) => integer_value(*value, Type::I32),
            Expression::Float(value) => float_value(value, Type::F64),
            Expression::Bool(value) => Ok(Value::Bool(*value)),
            Expression::Variable(name) => {
                variables.get(name).cloned().ok_or_else(|| CompileError {
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
    ) -> Result<Value, CompileError> {
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
                let value = variables.get(name).cloned().ok_or_else(|| CompileError {
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
    ) -> Result<(), CompileError> {
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
    ) -> Result<Value, CompileError> {
        if call.name == "println" {
            return Err(CompileError {
                message: "function `println` does not return a value".to_string(),
            });
        }

        self.call_function(&call.name, &call.arguments, variables)?
            .ok_or_else(|| CompileError {
                message: format!("function `{}` does not return a value", call.name),
            })
    }

    fn evaluate_println(
        &mut self,
        call: &CallExpression,
        variables: &HashMap<String, Value>,
    ) -> Result<(), CompileError> {
        if call.arguments.len() != 1 {
            return Err(CompileError {
                message: "function `println` expects 1 argument".to_string(),
            });
        }

        let value = self.evaluate(&call.arguments[0], variables)?;
        self.emitter.emit_puts(&value.printable());
        Ok(())
    }

    fn finish(self) -> String {
        self.emitter.finish()
    }
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

    fn finish(self) -> String {
        let mut output = String::new();
        output.push_str(".build_version macos, 11, 0\n");
        output.push_str(".section __TEXT,__cstring,cstring_literals\n");
        output.push_str(&self.cstrings);
        output.push('\n');
        output.push_str(".section __TEXT,__text,regular,pure_instructions\n\n");
        output.push_str(".globl _main\n");
        output.push_str(".p2align 2\n");
        output.push_str("_main:\n");
        output.push_str("    stp x29, x30, [sp, #-16]!\n");
        output.push_str("    mov x29, sp\n");
        output.push_str(&self.text);
        output.push_str("    mov w0, #0\n");
        output.push_str("    ldp x29, x30, [sp], #16\n");
        output.push_str("    ret\n\n");
        output.push_str(".subsections_via_symbols\n");
        output
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

fn expect_type(value: Value, ty: Type) -> Result<Value, CompileError> {
    if value.ty() == ty {
        Ok(value)
    } else {
        Err(CompileError {
            message: format!(
                "cannot use `{}` value as `{}`",
                value.ty().name(),
                ty.name()
            ),
        })
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
    fn emits_function_with_params_and_return() {
        let assembly = compile(
            "fn sample(a: i32, b: bool) -> i32 { return a } fn main() { println(sample(7, true)) }",
        );

        assert!(assembly.contains(".asciz \"7\""));
    }

    #[test]
    fn emits_void_function_with_params() {
        let assembly = compile("fn log(a: i32) { println(a) } fn main() { log(8) }");

        assert!(assembly.contains(".asciz \"8\""));
    }

    #[test]
    fn emits_u128() {
        let assembly = compile(
            "fn main() { let n: u128 = 340282366920938463463374607431768211455 println(n) }",
        );

        assert!(assembly.contains(".asciz \"340282366920938463463374607431768211455\""));
    }

    #[test]
    fn rejects_missing_main() {
        let error = compile_error("fn helper() {}");

        assert!(error.message.contains("function `main` not found"));
    }

    #[test]
    fn rejects_missing_return() {
        let error = compile_error("fn sample() -> i32 {} fn main() { sample() }");

        assert!(error.message.contains("must return `i32`"));
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
