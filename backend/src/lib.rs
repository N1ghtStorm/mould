pub mod compiler;

pub use compiler::{
    CompileError, compile_file, compile_file_to_assembly, compile_program_to_assembly,
    compile_program_to_executable, compile_source_to_assembly, compile_source_to_executable,
};
