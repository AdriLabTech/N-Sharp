//! Ejecuta cada `examples/*.ncs` y compara salida estándar y código de salida con `.out` / `.code`.
//! Además comprueba que cada `tests/invalid/*.ncs` es rechazado con el error indicado en su primera línea.

use std::path::{Path, PathBuf};
use std::process::Command;

fn ncom() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ncom"))
}

fn files(dir: &str, ext: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join(dir))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == ext))
        .collect();
    v.sort();
    v
}

#[test]
fn examples_produce_expected_output() {
    let mut failures = vec![];
    for src in files("examples", "ncs") {
        let expected = std::fs::read_to_string(src.with_extension("out")).unwrap_or_else(|_| panic!("falta {}", src.with_extension("out").display()));
        let code: i32 = std::fs::read_to_string(src.with_extension("code")).map(|s| s.trim().parse().unwrap()).unwrap_or(0);
        let out = ncom().arg("--release").arg(&src).output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        if stdout != expected || out.status.code() != Some(code) {
            failures.push(format!(
                "{}: código {:?} (esperado {code})\n--- salida ---\n{stdout}\n--- esperado ---\n{expected}\n--- stderr ---\n{}",
                src.display(),
                out.status.code(),
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n=====\n"));
}

#[test]
fn invalid_programs_are_rejected() {
    let mut failures = vec![];
    for src in files("tests/invalid", "ncs") {
        let text = std::fs::read_to_string(&src).unwrap();
        let expect = text.lines().next().and_then(|l| l.strip_prefix("# error: ")).unwrap_or_else(|| panic!("{} necesita `# error: ...` en la primera línea", src.display())).trim().to_string();
        let out = ncom().arg("check").arg(&src).output().unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        if out.status.success() || !stderr.contains(&expect) {
            failures.push(format!("{}: se esperaba el error «{expect}»\nstderr: {stderr}", src.display()));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n=====\n"));
}

#[test]
fn formatter_is_idempotent_and_preserves_behavior() {
    for src in files("examples", "ncs") {
        let text = std::fs::read_to_string(&src).unwrap();
        let once = nsharp::fmt::format_source(&text).unwrap_or_else(|_| panic!("no formatea {}", src.display()));
        let twice = nsharp::fmt::format_source(&once).unwrap();
        assert_eq!(once, twice, "fmt no es idempotente en {}", src.display());
        // el código formateado sigue produciendo la misma salida
        let tmp = std::env::temp_dir().join(format!("ncom_fmt_{}.ncs", src.file_stem().unwrap().to_string_lossy()));
        std::fs::write(&tmp, &once).unwrap();
        let a = ncom().arg("--release").arg(&src).output().unwrap();
        let b = ncom().arg("--release").arg(&tmp).output().unwrap();
        assert_eq!(a.stdout, b.stdout, "el formato cambió el comportamiento de {}", src.display());
        let _ = std::fs::remove_file(tmp);
    }
}
