use std::env;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::process::ExitCode;

mod convert;

fn print_usage(program: &str) {
    eprintln!("usage:");
    eprintln!("  {program} flatten [FILE]   indented tree -> pid,ppid,name lines");
    eprintln!("  {program} nest [FILE]      pid,ppid,name lines -> indented tree");
    eprintln!("  reads FILE if given, otherwise stdin; writes to stdout");
}

fn open_input(path: Option<&String>) -> io::Result<Box<dyn BufRead>> {
    match path {
        Some(p) => Ok(Box::new(BufReader::new(File::open(p)?))),
        None => Ok(Box::new(BufReader::new(io::stdin()))),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let program = args.first().map(|s| s.as_str()).unwrap_or("ptreeconv");

    let Some(command) = args.get(1) else {
        print_usage(program);
        return ExitCode::FAILURE;
    };

    let input_path = args.get(2);
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());

    let result = match command.as_str() {
        "flatten" => open_input(input_path).and_then(|r| convert::flatten(r, &mut out)),
        "nest" => open_input(input_path).and_then(|r| convert::nest(r, &mut out)),
        _ => {
            print_usage(program);
            return ExitCode::FAILURE;
        }
    };

    match result.and_then(|()| out.flush()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{program}: {e}");
            ExitCode::FAILURE
        }
    }
}
