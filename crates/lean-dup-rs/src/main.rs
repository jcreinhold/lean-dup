use std as standard;

fn main() {
    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    let code = lean_dup_rs::run(std::env::args_os(), &mut stdout, &mut stderr);
    standard::process::exit(code);
}
