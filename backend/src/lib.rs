pub mod compiler;

pub use compiler::{
    CompileError, compile_file, compile_program_to_c, compile_program_to_executable,
    compile_source_to_executable,
};
