use akuapkg_cli::entrypoint::run_from;

fn main() {
    std::process::exit(run_from(std::env::args_os()).code());
}
