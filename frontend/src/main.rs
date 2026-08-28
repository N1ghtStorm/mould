use std::{env, fs, process};

use frontend::parse_source;

fn main() {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: frontend <source-file>");
        process::exit(2);
    };

    let source = match fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("failed to read `{path}`: {error}");
            process::exit(1);
        }
    };

    match parse_source(&source) {
        Ok(program) => println!("parsed {} function(s)", program.functions.len()),
        Err(error) => {
            eprintln!(
                "parse error at {}..{}: {}",
                error.span.start, error.span.end, error.message
            );
            process::exit(1);
        }
    }
}
