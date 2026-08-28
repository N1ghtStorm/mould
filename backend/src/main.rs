use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{self, Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use backend::compile_file;

fn main() {
    let command = match parse_args(env::args().skip(1)) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("{message}");
            print_usage();
            process::exit(2);
        }
    };

    if let Err(message) = run(command) {
        eprintln!("{message}");
        process::exit(1);
    }
}

#[derive(Debug)]
enum CliCommand {
    Run {
        source_path: PathBuf,
    },
    Build {
        source_path: PathBuf,
        output_path: PathBuf,
    },
}

fn run(command: CliCommand) -> Result<(), String> {
    match command {
        CliCommand::Run { source_path } => run_file(&source_path),
        CliCommand::Build {
            source_path,
            output_path,
        } => {
            compile_file(&source_path, &output_path)
                .map_err(|error| format!("compile error: {}", error.message))?;
            println!("compiled {}", output_path.display());
            Ok(())
        }
    }
}

fn run_file(source_path: &Path) -> Result<(), String> {
    let output_path = temporary_executable_path();

    compile_file(source_path, &output_path)
        .map_err(|error| format!("compile error: {}", error.message))?;

    let status = Command::new(&output_path)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| format!("failed to run `{}`: {error}", output_path.display()));

    let _ = fs::remove_file(&output_path);

    match status {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("program exited with {status}")),
        Err(error) => Err(error),
    }
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<CliCommand, String> {
    let mut source_path = None;
    let mut output_path = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-o" | "--output" => {
                let Some(path) = args.next() else {
                    return Err("missing output file after `-o`".to_string());
                };

                output_path = Some(PathBuf::from(path));
            }
            "-h" | "--help" => {
                return Err("mould language cli".to_string());
            }
            _ if arg.starts_with('-') => {
                return Err(format!("unknown option `{arg}`"));
            }
            _ if source_path.is_none() => {
                source_path = Some(PathBuf::from(arg));
            }
            _ => {
                return Err(format!("unexpected argument `{arg}`"));
            }
        }
    }

    let Some(source_path) = source_path else {
        return Err("missing source file".to_string());
    };

    if source_path
        .extension()
        .and_then(|extension| extension.to_str())
        != Some("mould")
    {
        return Err("source file must have `.mould` extension".to_string());
    }

    if let Some(output_path) = output_path {
        return Ok(CliCommand::Build {
            source_path,
            output_path,
        });
    }

    Ok(CliCommand::Run { source_path })
}

fn print_usage() {
    eprintln!("usage: mould <source-file.mould>");
    eprintln!("       mould <source-file.mould> -o <output-file>");
}

fn temporary_executable_path() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let file_name = format!(
        "mould-run-{}-{nanos}{}",
        process::id(),
        env::consts::EXE_SUFFIX
    );

    env::temp_dir().join(file_name)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{CliCommand, parse_args};

    #[test]
    fn parses_run_command() {
        let command = parse_args(["samples/hello.mould".to_string()].into_iter()).unwrap();

        match command {
            CliCommand::Run { source_path } => {
                assert_eq!(source_path, PathBuf::from("samples/hello.mould"));
            }
            CliCommand::Build { .. } => panic!("expected run command"),
        }
    }

    #[test]
    fn parses_build_command() {
        let command = parse_args(
            [
                "samples/hello.mould".to_string(),
                "-o".to_string(),
                "hello".to_string(),
            ]
            .into_iter(),
        )
        .unwrap();

        match command {
            CliCommand::Build {
                source_path,
                output_path,
            } => {
                assert_eq!(source_path, PathBuf::from("samples/hello.mould"));
                assert_eq!(output_path, PathBuf::from("hello"));
            }
            CliCommand::Run { .. } => panic!("expected build command"),
        }
    }

    #[test]
    fn rejects_non_mould_file() {
        let error = parse_args(["samples/hello.txt".to_string()].into_iter()).unwrap_err();

        assert!(error.contains("`.mould`"));
    }
}
