use nsharp::check::Options;

fn main() {
    let path = std::env::args().nth(1).expect("uso: ncom archivo.ncs");
    let text = std::fs::read_to_string(&path).unwrap();
    let root = std::path::Path::new(&path).parent().map(|p| p.to_path_buf());
    let c = nsharp::compile(&path, &text, root, Options { require_main: true, ..Options::default() });
    eprint!("{}", c.render());
    if c.has_errors() { std::process::exit(1); }
    let code = std::thread::Builder::new().stack_size(1 << 30).spawn(move || {
        let prog: &'static nsharp::program::Program = Box::leak(Box::new(c.prog));
        let map: &'static nsharp::diag::SourceMap = Box::leak(Box::new(c.map));
        match nsharp::interp::Interp::new(prog, map, true, vec![]) {
            Ok(mut it) => it.run_main(),
            Err(e) => { eprintln!("error interno: {e}"); 70 }
        }
    }).unwrap().join().unwrap();
    std::process::exit(code);
}
