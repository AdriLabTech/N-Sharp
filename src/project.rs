//! Proyectos de N#: manifiesto `nsharp.toml`, dependencias, andamiaje y ejecutables empaquetados.

use crate::program::SourceConfig;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub enum TomlValue {
    Str(String),
    Int(i64),
    Bool(bool),
    Table(BTreeMap<String, TomlValue>),
}

impl TomlValue {
    pub fn as_str(&self) -> Option<&str> {
        if let TomlValue::Str(s) = self { Some(s) } else { None }
    }
}

/// Subconjunto de TOML suficiente para `nsharp.toml`: secciones, claves, strings, enteros,
/// booleanos y tablas en línea.
pub fn parse_toml(text: &str) -> Result<BTreeMap<String, TomlValue>, String> {
    let mut root: BTreeMap<String, TomlValue> = BTreeMap::new();
    let mut section: Option<String> = None;
    for (ln, raw) in text.lines().enumerate() {
        let line = strip_comment(raw).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            let name = name.trim().to_string();
            root.entry(name.clone()).or_insert_with(|| TomlValue::Table(BTreeMap::new()));
            section = Some(name);
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            return Err(format!("línea {}: se esperaba `clave = valor`", ln + 1));
        };
        let key = k.trim().trim_matches('"').to_string();
        let (val, rest) = parse_value(v.trim()).map_err(|e| format!("línea {}: {e}", ln + 1))?;
        if !rest.trim().is_empty() {
            return Err(format!("línea {}: texto inesperado tras el valor: `{}`", ln + 1, rest.trim()));
        }
        match &section {
            None => {
                root.insert(key, val);
            }
            Some(s) => {
                if let Some(TomlValue::Table(t)) = root.get_mut(s) {
                    t.insert(key, val);
                }
            }
        }
    }
    Ok(root)
}

fn strip_comment(line: &str) -> &str {
    let mut in_str = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '#' if !in_str => return &line[..i],
            _ => {}
        }
    }
    line
}

fn parse_value(s: &str) -> Result<(TomlValue, &str), String> {
    let s = s.trim_start();
    if let Some(rest) = s.strip_prefix('"') {
        let mut out = String::new();
        let mut chars = rest.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '"' => return Ok((TomlValue::Str(out), &rest[i + 1..])),
                '\\' => match chars.next() {
                    Some((_, 'n')) => out.push('\n'),
                    Some((_, 't')) => out.push('\t'),
                    Some((_, c)) => out.push(c),
                    None => break,
                },
                c => out.push(c),
            }
        }
        return Err("string sin cerrar".into());
    }
    if let Some(mut rest) = s.strip_prefix('{') {
        let mut t = BTreeMap::new();
        loop {
            rest = rest.trim_start();
            if let Some(r) = rest.strip_prefix('}') {
                return Ok((TomlValue::Table(t), r));
            }
            let Some((k, r)) = rest.split_once('=') else { return Err("tabla en línea mal formada".into()) };
            let (v, r2) = parse_value(r)?;
            t.insert(k.trim().trim_matches('"').to_string(), v);
            rest = r2.trim_start();
            if let Some(r) = rest.strip_prefix(',') {
                rest = r;
            }
        }
    }
    for (lit, val) in [("true", true), ("false", false)] {
        if let Some(rest) = s.strip_prefix(lit) {
            return Ok((TomlValue::Bool(val), rest));
        }
    }
    let end = s.find(|c: char| !(c.is_ascii_digit() || c == '-' || c == '_')).unwrap_or(s.len());
    if end > 0 {
        if let Ok(n) = s[..end].replace('_', "").parse::<i64>() {
            return Ok((TomlValue::Int(n), &s[end..]));
        }
    }
    Err(format!("valor no reconocido: `{s}`"))
}

#[derive(Debug, Clone)]
pub enum DepSrc {
    Git { url: String, rev: Option<String> },
    Path(String),
    Version(String),
}

#[derive(Debug, Clone)]
pub struct Dep {
    pub name: String,
    pub src: DepSrc,
}

#[derive(Debug, Clone)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub deps: Vec<Dep>,
    /// Política de warnings: categoría → "allow" | "warn" | "deny".
    pub warnings: BTreeMap<String, String>,
}

pub fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let t = parse_toml(text)?;
    let name = t.get("name").and_then(|v| v.as_str()).ok_or("falta `name` en nsharp.toml")?.to_string();
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
        return Err(format!("nombre de paquete no válido: `{name}`"));
    }
    let version = t.get("version").and_then(|v| v.as_str()).unwrap_or("0.1.0").to_string();
    if !is_semver(&version) {
        return Err(format!("`version` debe ser SemVer (p. ej. 1.2.3), no `{version}`"));
    }
    let mut deps = vec![];
    if let Some(TomlValue::Table(d)) = t.get("dependencies") {
        for (k, v) in d {
            if !valid_package_name(k) {
                return Err(format!("nombre de dependencia no válido: `{k}` (usa letras, dígitos, `_` o `-`)"));
            }
            let src = match v {
                TomlValue::Str(ver) => DepSrc::Version(ver.clone()),
                TomlValue::Table(tb) => {
                    if let Some(u) = tb.get("git").and_then(|v| v.as_str()) {
                        DepSrc::Git { url: u.to_string(), rev: tb.get("rev").and_then(|v| v.as_str()).map(String::from) }
                    } else if let Some(p) = tb.get("path").and_then(|v| v.as_str()) {
                        DepSrc::Path(p.to_string())
                    } else if let Some(v) = tb.get("version").and_then(|v| v.as_str()) {
                        DepSrc::Version(v.to_string())
                    } else {
                        return Err(format!("la dependencia `{k}` necesita `git`, `path` o `version`"));
                    }
                }
                _ => return Err(format!("dependencia `{k}` mal formada")),
            };
            deps.push(Dep { name: k.clone(), src });
        }
    }
    let mut warnings = BTreeMap::new();
    if let Some(TomlValue::Table(w)) = t.get("warnings") {
        for (k, v) in w {
            match v.as_str() {
                Some(l @ ("allow" | "warn" | "deny")) => {
                    warnings.insert(k.clone(), l.to_string());
                }
                _ => return Err(format!("[warnings] `{k}` debe ser \"allow\", \"warn\" o \"deny\"")),
            }
        }
    }
    Ok(Manifest { name, version, deps, warnings })
}

pub fn is_semver(v: &str) -> bool {
    let core = v.split(['-', '+']).next().unwrap_or("");
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

#[derive(Debug, Clone)]
pub struct Project {
    pub root: PathBuf,
    pub manifest: Manifest,
}

impl Project {
    pub fn src_dir(&self) -> PathBuf {
        self.root.join("src")
    }
    pub fn main_file(&self) -> PathBuf {
        self.src_dir().join("main.ncs")
    }
    pub fn target_dir(&self) -> PathBuf {
        self.root.join("target")
    }
    pub fn deps_dir(&self) -> PathBuf {
        self.root.join(".ncom").join("deps")
    }

    pub fn load(root: &Path) -> Result<Project, String> {
        let mf = root.join("nsharp.toml");
        let text = std::fs::read_to_string(&mf).map_err(|e| format!("no se pudo leer {}: {e}", mf.display()))?;
        let manifest = parse_manifest(&text).map_err(|e| format!("{}: {e}", mf.display()))?;
        Ok(Project { root: root.to_path_buf(), manifest })
    }

    /// Busca `nsharp.toml` subiendo desde `start`.
    pub fn discover(start: &Path) -> Option<PathBuf> {
        let mut cur = std::fs::canonicalize(start).ok()?;
        if cur.is_file() {
            cur.pop();
        }
        loop {
            if cur.join("nsharp.toml").is_file() {
                return Some(cur);
            }
            if !cur.pop() {
                return None;
            }
        }
    }

    /// Directorios `src` de las dependencias ya instaladas.
    pub fn dep_roots(&self) -> Result<Vec<(String, PathBuf)>, String> {
        let mut out = vec![];
        for d in &self.manifest.deps {
            let dir = match &d.src {
                DepSrc::Path(p) => self.root.join(p),
                DepSrc::Git { .. } => self.deps_dir().join(&d.name),
                DepSrc::Version(v) => {
                    return Err(format!(
                        "la dependencia `{}` = \"{v}\" requiere un registro de paquetes, que aún no existe; usa `{{ git = \"...\" }}` o `{{ path = \"...\" }}`",
                        d.name
                    ))
                }
            };
            if !dir.is_dir() {
                return Err(format!("la dependencia `{}` no está instalada (¿falta `ncom install`?)", d.name));
            }
            out.push((d.name.clone(), dir.join("src")));
        }
        Ok(out)
    }

    pub fn source_config(&self) -> Result<SourceConfig, String> {
        Ok(SourceConfig { root: Some(self.src_dir()), deps: self.dep_roots()?, memory: Default::default() })
    }

    /// Descarga las dependencias git en `.ncom/deps`. Devuelve líneas informativas.
    pub fn install(&self) -> Result<Vec<String>, String> {
        let mut log = vec![];
        let mut lock = String::from("# Generado por `ncom install`. No editar a mano.\n");
        for d in &self.manifest.deps {
            match &d.src {
                DepSrc::Path(p) => {
                    let dir = self.root.join(p);
                    if !dir.is_dir() {
                        return Err(format!("la dependencia `{}`: no existe el directorio {}", d.name, dir.display()));
                    }
                    log.push(format!("  {} (path {})", d.name, p));
                }
                DepSrc::Git { url, rev } => {
                    if url.starts_with('-') || rev.as_deref().is_some_and(|r| r.starts_with('-')) {
                        return Err(format!("la dependencia `{}` tiene una URL o revisión no válida", d.name));
                    }
                    let dir = self.deps_dir().join(&d.name);
                    std::fs::create_dir_all(self.deps_dir()).map_err(|e| e.to_string())?;
                    if dir.is_dir() {
                        run_git(&["-C", &dir.display().to_string(), "fetch", "--quiet", "--all"])?;
                    } else {
                        run_git(&["clone", "--quiet", "--", url, &dir.display().to_string()])?;
                    }
                    if let Some(r) = rev {
                        run_git(&["-C", &dir.display().to_string(), "checkout", "--quiet", r])?;
                    }
                    let head = git_out(&["-C", &dir.display().to_string(), "rev-parse", "HEAD"]).unwrap_or_default();
                    lock.push_str(&format!("[[package]]\nname = \"{}\"\ngit = \"{url}\"\ncommit = \"{}\"\n\n", d.name, head.trim()));
                    log.push(format!("  {} (git {url} @ {})", d.name, head.trim().chars().take(8).collect::<String>()));
                }
                DepSrc::Version(v) => {
                    return Err(format!("la dependencia `{}` = \"{v}\" requiere un registro de paquetes, que aún no existe; usa `git` o `path`", d.name));
                }
            }
        }
        std::fs::write(self.root.join("nsharp.lock"), lock).map_err(|e| e.to_string())?;
        Ok(log)
    }
}

fn run_git(args: &[&str]) -> Result<(), String> {
    let st = std::process::Command::new("git").args(args).status().map_err(|e| format!("no se pudo ejecutar git: {e}"))?;
    if st.success() { Ok(()) } else { Err(format!("`git {}` falló", args.join(" "))) }
}

fn git_out(args: &[&str]) -> Option<String> {
    let o = std::process::Command::new("git").args(args).output().ok()?;
    if o.status.success() { Some(String::from_utf8_lossy(&o.stdout).to_string()) } else { None }
}

pub fn valid_package_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') && !name.starts_with('-')
}

/// Crea un proyecto nuevo en `dir`.
pub fn scaffold(dir: &Path, name: &str) -> Result<(), String> {
    if !valid_package_name(name) {
        return Err(format!("nombre de proyecto no válido: `{name}` (usa letras, dígitos, `_` o `-`)"));
    }
    if dir.join("nsharp.toml").exists() {
        return Err(format!("ya existe un proyecto en {}", dir.display()));
    }
    std::fs::create_dir_all(dir.join("src")).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("nsharp.toml"), format!("name = \"{name}\"\nversion = \"0.1.0\"\n\n[dependencies]\n"))
        .map_err(|e| e.to_string())?;
    let main = dir.join("src/main.ncs");
    if !main.exists() {
        std::fs::write(&main, "fn main() -> nothing {\n    println(\"Hello, World!\");\n}\n").map_err(|e| e.to_string())?;
    }
    let gi = dir.join(".gitignore");
    if !gi.exists() {
        std::fs::write(gi, "/target\n/.ncom\n").map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ---------- ejecutables empaquetados ----------

const MAGIC: &[u8; 16] = b"NSHARP-BUNDLE-01";

#[derive(Debug, Clone, Default)]
pub struct Bundle {
    pub mode_project: bool,
    /// 0 = release, 1 = normal, 2 = debug.
    pub verbosity: u8,
    /// (nombre de módulo, nombre a mostrar, texto); el primero es `main`.
    pub modules: Vec<(String, String, String)>,
}

impl Bundle {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = vec![u8::from(self.mode_project), self.verbosity];
        out.extend((self.modules.len() as u32).to_le_bytes());
        for (a, b, c) in &self.modules {
            for s in [a, b, c] {
                out.extend((s.len() as u32).to_le_bytes());
                out.extend(s.as_bytes());
            }
        }
        out
    }

    pub fn from_bytes(b: &[u8]) -> Option<Bundle> {
        let mode_project = *b.first()? != 0;
        let verbosity = *b.get(1)?;
        let mut pos = 2;
        let rd_u32 = |pos: &mut usize| -> Option<usize> {
            let v = u32::from_le_bytes(b.get(*pos..*pos + 4)?.try_into().ok()?) as usize;
            *pos += 4;
            Some(v)
        };
        let n = rd_u32(&mut pos)?;
        let mut modules = vec![];
        for _ in 0..n {
            let mut parts = vec![];
            for _ in 0..3 {
                let len = rd_u32(&mut pos)?;
                parts.push(String::from_utf8(b.get(pos..pos + len)?.to_vec()).ok()?);
                pos += len;
            }
            let c = parts.pop()?;
            let bb = parts.pop()?;
            let a = parts.pop()?;
            modules.push((a, bb, c));
        }
        Some(Bundle { mode_project, verbosity, modules })
    }

    /// Escribe `exe` (copia del propio intérprete) + el paquete como cola.
    pub fn write_executable(&self, base_exe: &Path, out: &Path) -> Result<(), String> {
        let mut data = std::fs::read(base_exe).map_err(|e| format!("no se pudo leer el ejecutable base: {e}"))?;
        // si el propio ejecutable ya lleva un paquete, se recorta
        if let Some(len) = embedded_len(&data) {
            data.truncate(data.len() - len - 8 - MAGIC.len());
        }
        let payload = self.to_bytes();
        data.extend(&payload);
        data.extend((payload.len() as u64).to_le_bytes());
        data.extend(MAGIC);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(out, data).map_err(|e| format!("no se pudo escribir {}: {e}", out.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(out, std::fs::Permissions::from_mode(0o755));
        }
        Ok(())
    }

    /// Lee el paquete incrustado en el ejecutable actual, si existe.
    pub fn from_current_exe() -> Option<Bundle> {
        let exe = std::env::current_exe().ok()?;
        let data = std::fs::read(exe).ok()?;
        let len = embedded_len(&data)?;
        let end = data.len() - 8 - MAGIC.len();
        Bundle::from_bytes(&data[end - len..end])
    }
}

fn embedded_len(data: &[u8]) -> Option<usize> {
    if data.len() < 8 + MAGIC.len() || &data[data.len() - MAGIC.len()..] != MAGIC {
        return None;
    }
    let l = data.len() - MAGIC.len() - 8;
    let len = u64::from_le_bytes(data[l..l + 8].try_into().ok()?) as usize;
    if len > l { None } else { Some(len) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest() {
        let m = parse_manifest("name = \"example\"\nversion = \"1.0.0\"\n[dependencies]\nhttp = \"1.0\"\nmylib = { git = \"https://x/y\" }\nloc = { path = \"../loc\" }\n[warnings]\nunused-variable = \"allow\"\n").unwrap();
        assert_eq!(m.name, "example");
        assert_eq!(m.deps.len(), 3);
        assert_eq!(m.warnings["unused-variable"], "allow");
    }

    #[test]
    fn rejects_dependency_names_with_path_components() {
        assert!(parse_manifest("name = \"a\"\n[dependencies]\n\"../x\" = { path = \"p\" }\n").is_err());
    }

    #[test]
    fn bad_manifest() {
        assert!(parse_manifest("version = \"1.0.0\"").is_err());
        assert!(parse_manifest("name = \"a\"\nversion = \"x\"").is_err());
    }

    #[test]
    fn bundle_roundtrip() {
        let b = Bundle { mode_project: true, verbosity: 1, modules: vec![("main".into(), "m.ncs".into(), "fn main() -> nothing {}".into())] };
        let r = Bundle::from_bytes(&b.to_bytes()).unwrap();
        assert_eq!(r.modules[0].2, "fn main() -> nothing {}");
        assert!(r.mode_project);
    }
}
