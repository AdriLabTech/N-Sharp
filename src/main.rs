use nsharp::check::Options;

fn main() {
    let path = std::env::args().nth(1).expect("uso: ncom archivo.ncs");
    let text = std::fs::read_to_string(&path).unwrap();
    let root = std::path::Path::new(&path).parent().map(|p| p.to_path_buf());
    let c = nsharp::compile(&path, &text, root, Options { require_main: true, ..Options::default() });
    eprint!("{}", c.render());
    std::process::exit(if c.has_errors() { 1 } else { 0 });
}
