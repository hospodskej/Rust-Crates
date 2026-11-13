//! Sanitises an image file: `cargo run --example sanitize -- upload.jpg clean.jpg`.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, output] = args.as_slice() else {
        eprintln!("usage: sanitize <input> <output>");
        return ExitCode::FAILURE;
    };
    let bytes = match std::fs::read(input) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("{input}: {error}");
            return ExitCode::FAILURE;
        }
    };
    match spliceosome::sanitize(&bytes, &spliceosome::Limits::default()) {
        Ok(clean) => {
            let (width, height) = clean.dimensions();
            println!(
                "{input}: {} {width}x{height}, {} -> {} bytes",
                clean.format(),
                bytes.len(),
                clean.bytes().len()
            );
            if let Err(error) = std::fs::write(output, clean.bytes()) {
                eprintln!("{output}: {error}");
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{input}: {error}");
            ExitCode::FAILURE
        }
    }
}
