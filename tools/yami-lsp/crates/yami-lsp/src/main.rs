use std::process::ExitCode;

fn main() -> ExitCode {
    match yami_lsp::run_stdio() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            // stdout is reserved for the LSP protocol; diagnostics about the
            // transport itself go to stderr.
            eprintln!("yami-lsp: {error}");
            ExitCode::FAILURE
        }
    }
}
