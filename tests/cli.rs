//! Pruebas de la CLI `ncom`: proyectos, módulos, dependencias, build, fmt, test y REPL.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn ncom() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ncom"))
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ncom_test_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn run_ok(c: &mut Command) -> String {
    let out = c.output().unwrap();
    assert!(out.status.success(), "falló: {}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn new_run_build_clean() {
    let dir = tmp("project");
    run_ok(ncom().current_dir(&dir).args(["new", "demo"]));
    let proj = dir.join("demo");
    assert_eq!(run_ok(ncom().current_dir(&proj).arg("run")), "Hello, World!\n");
    let out = run_ok(ncom().current_dir(&proj).arg("build"));
    assert!(out.contains("compilado"));
    let exe = proj.join("target/ncom/demo");
    assert!(exe.is_file());
    let r = Command::new(&exe).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&r.stdout), "Hello, World!\n");
    run_ok(ncom().current_dir(&proj).arg("clean"));
    assert!(!proj.join("target").exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn built_executable_passes_args_and_exit_code() {
    let dir = tmp("exe");
    write(&dir, "p.ncs", "fn main() -> $int {\n    a := args();\n    println(a.length());\n    return 7;\n}\n");
    run_ok(ncom().current_dir(&dir).args(["build", "p.ncs", "-o", "prog"]));
    let r = Command::new(dir.join("prog")).args(["uno", "dos"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&r.stdout), "2\n");
    assert_eq!(r.status.code(), Some(7));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn modules_visibility_and_wildcards() {
    let dir = tmp("modules");
    write(&dir, "nsharp.toml", "name = \"m\"\nversion = \"1.0.0\"\n");
    write(&dir, "src/geometry.ncs", "pub struct P { pub x $int; hidden $int; }\npub fn mk(x $int) -> P { return P { x: x, hidden: 1 }; }\nfn secret() -> $int { return 1; }\npub fn open() -> $int { return secret(); }\n");
    write(&dir, "src/utils/text.ncs", "pub fn twice(s $string) -> $string { return s + s; }\n");
    write(&dir, "src/main.ncs", "import geometry.*;\nimport utils.text;\nimport utils.text as t;\nfn main() -> nothing {\n    p := mk(4);\n    println(p.x);\n    println(open());\n    println(text.twice(\"ab\"));\n    println(t.twice(\"c\"));\n}\n");
    assert_eq!(run_ok(ncom().current_dir(&dir).arg("run")), "4\n1\nabab\ncc\n");
    write(&dir, "src/main.ncs", "import geometry;\nfn main() -> nothing {\n    println(geometry.secret());\n}\n");
    let out = ncom().current_dir(&dir).arg("run").output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("`secret` es privado"));
    write(&dir, "src/main.ncs", "import geometry;\nfn main() -> nothing {\n    p := geometry.mk(1);\n    println(p.hidden);\n}\n");
    let out = ncom().current_dir(&dir).arg("run").output().unwrap();
    assert!(String::from_utf8_lossy(&out.stderr).contains("el campo `hidden` de `P` es privado"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn project_rejects_global_code_with_main() {
    let dir = tmp("globals");
    write(&dir, "nsharp.toml", "name = \"g\"\n");
    write(&dir, "src/main.ncs", "println(\"x\");\n");
    let out = ncom().current_dir(&dir).arg("run").output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("código global solo se permite en script mode"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn script_without_main_is_an_error_on_run() {
    let dir = tmp("nomain");
    write(&dir, "a.ncs", "fn helper() -> nothing {}\n");
    let out = ncom().arg(dir.join("a.ncs")).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no se encontró la función `main`"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn path_dependencies() {
    let dir = tmp("deps");
    write(&dir, "lib/nsharp.toml", "name = \"mylib\"\n");
    write(&dir, "lib/src/lib.ncs", "pub fn greet(n $string) -> $string { return \"hola \" + n; }\n");
    write(&dir, "app/nsharp.toml", "name = \"app\"\n[dependencies]\nmylib = { path = \"../lib\" }\n");
    write(&dir, "app/src/main.ncs", "import mylib;\nfn main() -> nothing {\n    println(mylib.greet(\"mundo\"));\n}\n");
    assert_eq!(run_ok(ncom().current_dir(dir.join("app")).arg("run")), "hola mundo\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_dependencies_are_explained() {
    let dir = tmp("registry");
    write(&dir, "nsharp.toml", "name = \"r\"\n[dependencies]\nhttp = \"1.0\"\n");
    write(&dir, "src/main.ncs", "fn main() -> nothing {}\n");
    let out = ncom().current_dir(&dir).arg("run").output().unwrap();
    assert!(String::from_utf8_lossy(&out.stderr).contains("requiere un registro de paquetes"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn test_command_reports_failures() {
    let dir = tmp("tests");
    write(&dir, "t.ncs", "test \"ok\" { assert_eq(1 + 1, 2); }\ntest \"bad\" { assert_eq(1, 2); }\nfn main() -> nothing {}\n");
    let out = ncom().arg("test").arg(dir.join("t.ncs")).output().unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("test ok ... ok"), "{s}");
    assert!(s.contains("test bad ... FALLÓ"), "{s}");
    assert_eq!(out.status.code(), Some(1));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bench_command() {
    let dir = tmp("bench");
    write(&dir, "b.ncs", "bench \"sum\" { t := 0; loop (i := 0; i < 100; i++) { t += i; } }\nfn main() -> nothing {}\n");
    let out = run_ok(ncom().args(["test", "--bench"]).arg(dir.join("b.ncs")));
    assert!(out.contains("bench sum ..."), "{out}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn fmt_check_and_rewrite() {
    let dir = tmp("fmt");
    write(&dir, "f.ncs", "fn  main( )->nothing{println( 1+2 );}\n");
    let out = ncom().args(["fmt", "--check"]).arg(dir.join("f.ncs")).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    run_ok(ncom().arg("fmt").arg(dir.join("f.ncs")));
    assert_eq!(std::fs::read_to_string(dir.join("f.ncs")).unwrap(), "fn main() -> nothing {\n    println(1 + 2);\n}\n");
    run_ok(ncom().args(["fmt", "--check"]).arg(dir.join("f.ncs")));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn doc_generates_html() {
    let dir = tmp("doc");
    write(&dir, "nsharp.toml", "name = \"d\"\n");
    write(&dir, "src/main.ncs", "/// Suma dos números.\npub fn add(a $int, b $int) -> $int { return a + b; }\nfn main() -> nothing {}\n");
    run_ok(ncom().current_dir(&dir).arg("doc"));
    let html = std::fs::read_to_string(dir.join("target/doc/main.html")).unwrap();
    assert!(html.contains("Suma dos números."));
    assert!(html.contains("pub fn add(a $int, b $int) -&gt; $int"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn warnings_are_configurable() {
    let dir = tmp("warn");
    write(&dir, "w.ncs", "fn main() -> nothing { let unused = 1; }\n");
    let out = ncom().arg(dir.join("w.ncs")).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stderr).contains("nunca se usa"));
    let out = ncom().args(["-A", "unused-variable"]).arg(dir.join("w.ncs")).output().unwrap();
    assert!(!String::from_utf8_lossy(&out.stderr).contains("nunca se usa"));
    let out = ncom().args(["-D", "unused-variable"]).arg(dir.join("w.ncs")).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn panic_modes() {
    let dir = tmp("panic");
    write(&dir, "p.ncs", "fn f() -> nothing { panic(\"boom\"); }\nfn main() -> nothing { f(); }\n");
    let normal = ncom().arg(dir.join("p.ncs")).output().unwrap();
    let e = String::from_utf8_lossy(&normal.stderr);
    assert!(e.contains("panic: boom") && e.contains("p.ncs:1"), "{e}");
    assert_eq!(normal.status.code(), Some(101));
    let release = ncom().arg("--release").arg(dir.join("p.ncs")).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&release.stderr), "panic: boom\n");
    let debug = ncom().arg("--debug").arg(dir.join("p.ncs")).output().unwrap();
    assert!(String::from_utf8_lossy(&debug.stderr).contains("backtrace"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn repl_keeps_state() {
    let mut child = ncom().arg("repl").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"x := 5\nfn dbl(n $int) -> $int { return n * 2; }\ndbl(x) + 1\nx = 20\ndbl(x)\n\"hi\"\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "11\n40\n\"hi\"\n");
}

#[test]
fn script_reads_stdin() {
    let dir = tmp("stdin");
    write(&dir, "s.ncs", "name := input();\nprintln(\"hola {name}\");\n");
    let mut child = ncom().arg(dir.join("s.ncs")).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(b"Ana\n").unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "hola Ana\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn file_io() {
    let dir = tmp("io");
    let f = dir.join("out.txt");
    write(&dir, "io.ncs", &format!("fn main() -> nothing {{\n    writeFile(\"{p}\", \"datos\");\n    match readFile(\"{p}\") {{\n        Ok(t) => println(t);\n        Error(e) => println(\"error\");\n    }}\n    match readFile(\"{p}.nope\") {{\n        Ok(t) => println(t);\n        Error(e) => println(\"sin fichero\");\n    }}\n}}\n", p = f.display()));
    assert_eq!(run_ok(ncom().arg(dir.join("io.ncs"))), "datos\nsin fichero\n");
    let _ = std::fs::remove_dir_all(dir);
}
