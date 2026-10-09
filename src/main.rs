fn main() {
    let code = match protbot::run() {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("protbot: {err}");
            1
        }
    };
    std::process::exit(code);
}
