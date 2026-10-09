//! La CLI `ncom`.

use crate::check::{Mode, Options, WARNING_CATEGORIES};
use crate::diag;
use crate::interp::Interp;
use crate::program::SourceConfig;
use crate::project::{self, Bundle, Project};
use crate::{compile, Compiled};
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
ncom — NCompiler, la CLI de N# (N Sharp) v{VERSION}

USO:
    ncom [opciones] archivo.ncs [args...]   Compila y ejecuta un script
    ncom <comando> [opciones] [destino]

COMANDOS:
    build      Compila el proyecto o archivo a un ejecutable autónomo
    run        Compila y ejecuta (proyecto o archivo)
    check      Analiza el código sin ejecutarlo
    fmt        Formatea el código con el estilo oficial (--check para solo comprobar)
    test       Ejecuta los bloques `test` (--bench para los `bench`)
    clean      Elimina el directorio target/
    init       Crea un proyecto en el directorio actual
    new        Crea un proyecto nuevo: ncom new <nombre>
    install    Instala las dependencias de nsharp.toml
    repl       Abre el intérprete interactivo
    doc        Genera la documentación HTML (target/doc)

OPCIONES:
    --debug              Diagnósticos y backtraces detallados
    --release            Mensajes mínimos de abort/runtime
    -A, --allow <cat>    Silencia una categoría de warning
    -D, --deny <cat>     Convierte una categoría de warning en error
    -o <ruta>            Salida de `build` / `doc`
    -V, --version        Muestra la versión
    -h, --help           Muestra esta ayuda

CATEGORÍAS DE WARNING: {CATS}
";

#[derive(Default)]
struct Flags {
    verbosity: Option<u8>,
    allow: HashSet<String>,
    deny: HashSet<String>,
    out: Option<PathBuf>,
    check_only: bool,
    bench: bool,
    private: bool,
    with_std: bool,
    rest: Vec<String>,
}

fn print_help() {
    print!("{}", HELP.replace("{VERSION}", VERSION).replace("{CATS}", &WARNING_CATEGORIES.join(", ")));
}

fn parse_flags(args: &[String]) -> Result<Flags, String> {
    let mut f = Flags::default();
    let mut i = 0;
    let mut after_dd = false;
    while i < args.len() {
        let a = &args[i];
        if after_dd {
            f.rest.push(a.clone());
        } else {
            match a.as_str() {
                "--" => after_dd = true,
                "--debug" => f.verbosity = Some(2),
                "--release" => f.verbosity = Some(0),
                "-A" | "--allow" | "-D" | "--deny" => {
                    i += 1;
                    let cat = args.get(i).ok_or_else(|| format!("`{a}` necesita una categoría"))?;
                    if cat != "warnings" && !WARNING_CATEGORIES.contains(&cat.as_str()) {
                        return Err(format!("categoría de warning desconocida: `{cat}` (disponibles: {}, warnings)", WARNING_CATEGORIES.join(", ")));
                    }
                    let set = if a == "-A" || a == "--allow" { &mut f.allow } else { &mut f.deny };
                    if cat == "warnings" {
                        set.extend(WARNING_CATEGORIES.iter().map(|s| s.to_string()));
                    } else {
                        set.insert(cat.clone());
                    }
                }
                "-o" | "--output" => {
                    i += 1;
                    f.out = Some(PathBuf::from(args.get(i).ok_or("`-o` necesita una ruta")?));
                }
                "--check" => f.check_only = true,
                "--bench" => f.bench = true,
                "--private" => f.private = true,
                "--std" => f.with_std = true,
                _ => f.rest.push(a.clone()),
            }
        }
        i += 1;
    }
    Ok(f)
}

enum Target {
    Project(Project),
    Script(PathBuf),
}

fn resolve_target(arg: Option<&str>) -> Result<Target, String> {
    match arg {
        None => {
            let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
            match Project::discover(&cwd) {
                Some(root) => Ok(Target::Project(Project::load(&root)?)),
                None => Err("no se encontró nsharp.toml; indica un archivo `.ncs` o crea un proyecto con `ncom init`".into()),
            }
        }
        Some(p) => {
            let path = PathBuf::from(p);
            if path.is_dir() {
                if path.join("nsharp.toml").is_file() {
                    Ok(Target::Project(Project::load(&path)?))
                } else {
                    Err(format!("{p} no es un proyecto (falta nsharp.toml)"))
                }
            } else if path.file_name().is_some_and(|n| n == "nsharp.toml") {
                Ok(Target::Project(Project::load(path.parent().filter(|x| !x.as_os_str().is_empty()).unwrap_or(Path::new(".")))?))
            } else if path.is_file() {
                Ok(Target::Script(path))
            } else {
                Err(format!("no existe: {p}"))
            }
        }
    }
}

fn options(flags: &Flags, mode: Mode, require_main: bool, manifest: Option<&project::Manifest>) -> Options {
    let mut allow = flags.allow.clone();
    let mut deny = flags.deny.clone();
    if let Some(m) = manifest {
        for (k, v) in &m.warnings {
            let cats: Vec<String> = if k == "warnings" { WARNING_CATEGORIES.iter().map(|s| s.to_string()).collect() } else { vec![k.clone()] };
            for c in cats {
                match v.as_str() {
                    "allow" => { allow.insert(c); }
                    "deny" => { deny.insert(c); }
                    _ => {}
                }
            }
        }
    }
    // la línea de comandos tiene prioridad sobre el manifiesto
    for c in &flags.deny { allow.remove(c); }
    for c in &flags.allow { deny.remove(c); }
    Options { mode, allow, deny, require_main }
}

struct Built {
    c: Compiled,
    name: String,
    project: Option<Project>,
}

fn build_one(file: &Path, cfg: SourceConfig, opts: Options) -> Result<Compiled, String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("no se pudo leer {}: {e}", file.display()))?;
    Ok(compile(&file.display().to_string(), &text, cfg, opts))
}

fn load_target(target: &Target, flags: &Flags, require_main: bool) -> Result<Built, String> {
    match target {
        Target::Script(path) => {
            let cfg = SourceConfig { root: path.parent().filter(|p| !p.as_os_str().is_empty()).map(|p| p.to_path_buf()).or(Some(PathBuf::from("."))), ..Default::default() };
            let c = build_one(path, cfg, options(flags, Mode::Script, require_main, None))?;
            let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "a".into());
            Ok(Built { c, name, project: None })
        }
        Target::Project(p) => {
            let main = p.main_file();
            if !main.is_file() {
                return Err(format!("falta {} (punto de entrada del proyecto)", main.display()));
            }
            let c = build_one(&main, p.source_config()?, options(flags, Mode::Project, require_main, Some(&p.manifest)))?;
            Ok(Built { c, name: p.manifest.name.clone(), project: Some(p.clone()) })
        }
    }
}

/// Imprime los diagnósticos; devuelve true si hay errores.
fn report(c: &Compiled, verbosity: u8) -> bool {
    let shown: Vec<_> = c.diags.iter().filter(|d| d.is_error() || verbosity > 0).cloned().collect();
    if !shown.is_empty() {
        let _ = std::io::stdout().flush();
        eprint!("{}", diag::render_all(&shown, &c.map));
        let errors = shown.iter().filter(|d| d.is_error()).count();
        let warns = shown.len() - errors;
        if errors > 0 {
            eprintln!("\nerror: no se pudo compilar ({errors} error(es){})", if warns > 0 { format!(", {warns} warning(s)") } else { String::new() });
        } else if verbosity > 0 {
            eprintln!("\n{warns} warning(s)");
        }
    }
    c.has_errors()
}

/// Ejecuta el programa en un hilo con pila grande. Devuelve el código de salida.
pub fn execute(c: Compiled, verbosity: u8, args: Vec<String>) -> i32 {
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || {
            let prog: &'static crate::program::Program = Box::leak(Box::new(c.prog));
            let map: &'static diag::SourceMap = Box::leak(Box::new(c.map));
            match Interp::new(prog, map, verbosity, args) {
                Ok(mut it) => it.run_main(),
                Err(e) => {
                    eprintln!("error interno: {e}");
                    70
                }
            }
        })
        .expect("no se pudo crear el hilo de ejecución")
        .join()
        .unwrap_or(70)
}

fn cmd_run(flags: &Flags) -> i32 {
    let (target_arg, prog_args) = match flags.rest.split_first() {
        Some((t, rest)) => (Some(t.as_str()), rest.to_vec()),
        None => (None, vec![]),
    };
    let target = match resolve_target(target_arg) {
        Ok(t) => t,
        Err(e) => return fail(&e),
    };
    let verbosity = flags.verbosity.unwrap_or(1);
    let built = match load_target(&target, flags, true) {
        Ok(b) => b,
        Err(e) => return fail(&e),
    };
    if report(&built.c, verbosity) {
        return 1;
    }
    execute(built.c, verbosity, prog_args)
}

fn fail(msg: &str) -> i32 {
    eprintln!("error: {msg}");
    1
}

fn cmd_build(flags: &Flags) -> i32 {
    let target = match resolve_target(flags.rest.first().map(|s| s.as_str())) {
        Ok(t) => t,
        Err(e) => return fail(&e),
    };
    let verbosity = flags.verbosity.unwrap_or(1);
    let built = match load_target(&target, flags, true) {
        Ok(b) => b,
        Err(e) => return fail(&e),
    };
    if report(&built.c, verbosity) {
        return 1;
    }
    let mut modules = vec![];
    for m in built.c.prog.modules.iter().filter(|m| !m.is_std) {
        let f = built.c.map.get(m.file);
        modules.push((m.name.clone(), f.name.clone(), f.text.clone()));
    }
    let bundle = Bundle { mode_project: built.project.is_some(), verbosity, modules };
    let out = match (&flags.out, &built.project) {
        (Some(o), _) => o.clone(),
        (None, Some(p)) => p.target_dir().join("ncom").join(&built.name),
        (None, None) => PathBuf::from(&built.name),
    };
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => return fail(&format!("no se pudo localizar el ejecutable de ncom: {e}")),
    };
    match bundle.write_executable(&exe, &out) {
        Ok(()) => {
            println!("compilado: {}", out.display());
            0
        }
        Err(e) => fail(&e),
    }
}

fn collect_ncs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if p.is_dir() {
            if !name.starts_with('.') && name != "target" {
                collect_ncs(&p, out);
            }
        } else if p.extension().is_some_and(|e| e == "ncs") {
            out.push(p);
        }
    }
}

fn cmd_check(flags: &Flags) -> i32 {
    let target = match resolve_target(flags.rest.first().map(|s| s.as_str())) {
        Ok(t) => t,
        Err(e) => return fail(&e),
    };
    let verbosity = flags.verbosity.unwrap_or(1);
    let mut errors = false;
    match &target {
        Target::Script(_) => {
            let built = match load_target(&target, flags, false) {
                Ok(b) => b,
                Err(e) => return fail(&e),
            };
            errors |= report(&built.c, verbosity);
        }
        Target::Project(p) => {
            let mut files = vec![];
            collect_ncs(&p.src_dir(), &mut files);
            let mut seen: HashSet<String> = HashSet::new();
            for f in files {
                let is_main = f == p.main_file();
                let cfg = match p.source_config() {
                    Ok(c) => c,
                    Err(e) => return fail(&e),
                };
                let opts = options(flags, Mode::Project, is_main, Some(&p.manifest));
                let c = match build_one(&f, cfg, opts) {
                    Ok(c) => c,
                    Err(e) => return fail(&e),
                };
                // evita repetir los mismos diagnósticos de módulos compartidos
                let mut c = c;
                c.diags.retain(|d| seen.insert(format!("{:?}{}{:?}", d.span, d.message, c.map.files.get(d.span.map(|s| s.file as usize).unwrap_or(usize::MAX)).map(|f| f.name.clone()))));
                errors |= report(&c, verbosity);
            }
        }
    }
    if !errors {
        println!("sin errores");
    }
    i32::from(errors)
}

fn cmd_test(flags: &Flags) -> i32 {
    let mut rest = flags.rest.clone();
    let target_arg = if rest.first().is_some_and(|a| Path::new(a).exists()) { Some(rest.remove(0)) } else { None };
    let filter = rest.first().cloned();
    let target = match resolve_target(target_arg.as_deref()) {
        Ok(t) => t,
        Err(e) => return fail(&e),
    };
    let verbosity = flags.verbosity.unwrap_or(1);
    let mut entries: Vec<(PathBuf, SourceConfig, Options)> = vec![];
    match &target {
        Target::Script(p) => {
            let cfg = SourceConfig { root: p.parent().filter(|x| !x.as_os_str().is_empty()).map(|x| x.to_path_buf()).or(Some(PathBuf::from("."))), ..Default::default() };
            entries.push((p.clone(), cfg, options(flags, Mode::Script, false, None)));
        }
        Target::Project(p) => {
            let mut files = vec![];
            collect_ncs(&p.src_dir(), &mut files);
            for f in files {
                let cfg = match p.source_config() {
                    Ok(c) => c,
                    Err(e) => return fail(&e),
                };
                entries.push((f.clone(), cfg, options(flags, Mode::Project, false, Some(&p.manifest))));
            }
        }
    }
    let (mut total_ok, mut total_fail) = (0, 0);
    for (file, cfg, opts) in entries {
        let c = match build_one(&file, cfg, opts) {
            Ok(c) => c,
            Err(e) => return fail(&e),
        };
        if report(&c, verbosity.min(1)) {
            return 1;
        }
        let has_any = c.prog.modules[c.prog.main].ast.items.iter().any(|it| matches!(it.kind, crate::ast::ItemKind::Test { .. } | crate::ast::ItemKind::Bench { .. }));
        if !has_any {
            continue;
        }
        let filter = filter.clone();
        let bench = flags.bench;
        let (ok, bad) = std::thread::Builder::new()
            .stack_size(1 << 30)
            .spawn(move || {
                let prog: &'static crate::program::Program = Box::leak(Box::new(c.prog));
                let map: &'static diag::SourceMap = Box::leak(Box::new(c.map));
                match Interp::new(prog, map, verbosity, vec![]) {
                    Ok(mut it) => {
                        it.only_main_tests = true;
                        let r = if bench { (0, it.run_benches(filter.as_deref())) } else { it.run_tests(filter.as_deref()) };
                        it.flush();
                        r
                    }
                    Err(e) => {
                        eprintln!("error interno: {e}");
                        (0, 1)
                    }
                }
            })
            .expect("hilo")
            .join()
            .unwrap_or((0, 1));
        total_ok += ok;
        total_fail += bad;
    }
    if total_ok + total_fail == 0 {
        println!("no se encontraron tests");
    }
    i32::from(total_fail > 0)
}

fn cmd_fmt(flags: &Flags) -> i32 {
    let mut files: Vec<PathBuf> = vec![];
    if flags.rest.iter().any(|a| a == "-") {
        let mut src = String::new();
        let _ = std::io::Read::read_to_string(&mut std::io::stdin(), &mut src);
        return match crate::fmt::format_source(&src) {
            Ok(out) => {
                print!("{out}");
                0
            }
            Err(ds) => {
                let mut map = diag::SourceMap::default();
                map.add("<stdin>", src);
                eprint!("{}", diag::render_all(&ds, &map));
                1
            }
        };
    }
    if flags.rest.is_empty() {
        let cwd = std::env::current_dir().unwrap_or_default();
        match Project::discover(&cwd) {
            Some(root) => collect_ncs(&root.join("src"), &mut files),
            None => collect_ncs(&cwd, &mut files),
        }
    } else {
        for a in &flags.rest {
            let p = PathBuf::from(a);
            if p.is_dir() {
                collect_ncs(&p, &mut files);
            } else if p.is_file() {
                files.push(p);
            } else {
                return fail(&format!("no existe: {a}"));
            }
        }
    }
    if files.is_empty() {
        println!("no hay archivos .ncs que formatear");
        return 0;
    }
    let (mut changed, mut bad) = (0, 0);
    for f in &files {
        let src = match std::fs::read_to_string(f) {
            Ok(s) => s,
            Err(e) => return fail(&format!("{}: {e}", f.display())),
        };
        match crate::fmt::format_source(&src) {
            Ok(out) => {
                if out != src {
                    changed += 1;
                    if flags.check_only {
                        println!("necesita formato: {}", f.display());
                    } else if let Err(e) = std::fs::write(f, out) {
                        return fail(&format!("{}: {e}", f.display()));
                    } else {
                        println!("formateado: {}", f.display());
                    }
                }
            }
            Err(ds) => {
                bad += 1;
                let mut map = diag::SourceMap::default();
                map.add(f.display().to_string(), src);
                eprint!("{}", diag::render_all(&ds, &map));
            }
        }
    }
    if bad > 0 {
        return 1;
    }
    if flags.check_only {
        if changed > 0 {
            eprintln!("{changed} archivo(s) sin formatear");
            return 1;
        }
        println!("todo está formateado");
    } else if changed == 0 {
        println!("todo estaba formateado ({} archivo(s))", files.len());
    }
    0
}

fn cmd_clean() -> i32 {
    let cwd = std::env::current_dir().unwrap_or_default();
    let Some(root) = Project::discover(&cwd) else { return fail("no se encontró nsharp.toml") };
    let target = root.join("target");
    if target.is_dir() {
        match std::fs::remove_dir_all(&target) {
            Ok(()) => println!("eliminado {}", target.display()),
            Err(e) => return fail(&format!("{}: {e}", target.display())),
        }
    } else {
        println!("nada que limpiar");
    }
    0
}

fn cmd_init() -> i32 {
    let cwd = std::env::current_dir().unwrap_or_default();
    let name = cwd.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "proyecto".into());
    match project::scaffold(&cwd, &name) {
        Ok(()) => {
            println!("proyecto `{name}` creado en {}", cwd.display());
            0
        }
        Err(e) => fail(&e),
    }
}

fn cmd_new(flags: &Flags) -> i32 {
    let Some(name) = flags.rest.first() else { return fail("uso: ncom new <nombre>") };
    let dir = PathBuf::from(name);
    let pkg = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| name.clone());
    match project::scaffold(&dir, &pkg) {
        Ok(()) => {
            println!("proyecto `{pkg}` creado en {}", dir.display());
            println!("  cd {} && ncom run", dir.display());
            0
        }
        Err(e) => fail(&e),
    }
}

fn cmd_install() -> i32 {
    let cwd = std::env::current_dir().unwrap_or_default();
    let Some(root) = Project::discover(&cwd) else { return fail("no se encontró nsharp.toml") };
    let project = match Project::load(&root) {
        Ok(p) => p,
        Err(e) => return fail(&e),
    };
    if project.manifest.deps.is_empty() {
        println!("sin dependencias");
        return 0;
    }
    match project.install() {
        Ok(log) => {
            println!("dependencias instaladas:");
            for l in log {
                println!("{l}");
            }
            0
        }
        Err(e) => fail(&e),
    }
}

fn cmd_doc(flags: &Flags) -> i32 {
    let target = match resolve_target(flags.rest.first().map(|s| s.as_str())) {
        Ok(t) => t,
        Err(e) => return fail(&e),
    };
    let built = match load_target(&target, flags, false) {
        Ok(b) => b,
        Err(e) => return fail(&e),
    };
    if report(&built.c, 0) {
        return 1;
    }
    let out = match (&flags.out, &built.project) {
        (Some(o), _) => o.clone(),
        (None, Some(p)) => p.target_dir().join("doc"),
        (None, None) => PathBuf::from("doc"),
    };
    match crate::doc::generate(&built.c.prog, &built.name, &out, flags.with_std, flags.private) {
        Ok(files) => {
            println!("documentación generada en {} ({} páginas)", out.display(), files.len());
            0
        }
        Err(e) => fail(&e),
    }
}

/// Ejecuta un ejecutable compilado con `ncom build`.
pub fn run_bundle(b: Bundle, args: Vec<String>) -> i32 {
    let mut cfg = SourceConfig::default();
    for (name, display, text) in b.modules.iter().skip(1) {
        cfg.memory.insert(name.clone(), (display.clone(), text.clone()));
    }
    let (_, display, text) = &b.modules[0];
    let opts = Options { mode: if b.mode_project { Mode::Project } else { Mode::Script }, require_main: true, ..Options::default() };
    let c = compile(display, text, cfg, opts);
    if report(&c, 0) {
        return 1;
    }
    execute(c, b.verbosity, args)
}

pub fn main_with_args(args: Vec<String>) -> i32 {
    if let Some(b) = Bundle::from_current_exe() {
        return run_bundle(b, args);
    }
    let Some(first) = args.first() else {
        print_help();
        return 0;
    };
    match first.as_str() {
        "-h" | "--help" | "help" => {
            print_help();
            return 0;
        }
        "-V" | "--version" | "version" => {
            println!("ncom {VERSION} (N# v0.1)");
            return 0;
        }
        _ => {}
    }
    let commands = ["build", "run", "check", "fmt", "test", "clean", "init", "new", "install", "repl", "doc"];
    let (cmd, rest): (&str, &[String]) = if commands.contains(&first.as_str()) { (first.as_str(), &args[1..]) } else { ("run", &args[..]) };
    // en modo script `ncom [opciones] archivo.ncs args...` lo que sigue al archivo son argumentos del programa
    let flags = if cmd == "run" && !commands.contains(&first.as_str()) || cmd == "run" {
        let split = rest.iter().position(|a| !a.starts_with('-') && (Path::new(a).exists() || a.ends_with(".ncs")));
        let (opts_part, tail) = match split {
            Some(i) => (&rest[..=i], &rest[i + 1..]),
            None => (rest, &rest[rest.len()..]),
        };
        let mut f = match parse_flags(opts_part) {
            Ok(f) => f,
            Err(e) => return fail(&e),
        };
        let mut tail: Vec<String> = tail.to_vec();
        if tail.first().is_some_and(|a| a == "--") {
            tail.remove(0);
        }
        f.rest.extend(tail);
        f
    } else {
        match parse_flags(rest) {
            Ok(f) => f,
            Err(e) => return fail(&e),
        }
    };
    match cmd {
        "run" => cmd_run(&flags),
        "build" => cmd_build(&flags),
        "check" => cmd_check(&flags),
        "fmt" => cmd_fmt(&flags),
        "test" => cmd_test(&flags),
        "clean" => cmd_clean(),
        "init" => cmd_init(),
        "new" => cmd_new(&flags),
        "install" => cmd_install(),
        "repl" => crate::repl::run(),
        "doc" => cmd_doc(&flags),
        _ => unreachable!(),
    }
}
