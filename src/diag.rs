//! Spans, mapa de fuentes y diagnósticos con contexto de código.

use std::fmt::Write as _;
use std::io::IsTerminal;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Span {
    pub file: u32,
    pub lo: u32,
    pub hi: u32,
}

impl Span {
    pub fn new(file: u32, lo: usize, hi: usize) -> Span {
        Span { file, lo: lo as u32, hi: hi as u32 }
    }
    pub fn to(self, other: Span) -> Span {
        Span { file: self.file, lo: self.lo.min(other.lo), hi: self.hi.max(other.hi) }
    }
}

pub struct SourceFile {
    pub name: String,
    pub text: String,
    line_starts: Vec<usize>,
}

impl SourceFile {
    fn new(name: String, text: String) -> Self {
        let mut line_starts = vec![0];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        SourceFile { name, text, line_starts }
    }

    /// Línea (1-based) y columna (1-based, en caracteres) de un offset.
    pub fn line_col(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.text.len());
        let line = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let start = self.line_starts[line];
        let col = self.text[start..offset].chars().count() + 1;
        (line + 1, col)
    }

    pub fn line_text(&self, line: usize) -> &str {
        let start = self.line_starts[line - 1];
        let end = self.line_starts.get(line).copied().unwrap_or(self.text.len());
        self.text[start..end].trim_end_matches(['\n', '\r'])
    }
}

#[derive(Default)]
pub struct SourceMap {
    pub files: Vec<SourceFile>,
}

impl SourceMap {
    pub fn add(&mut self, name: impl Into<String>, text: impl Into<String>) -> u32 {
        self.files.push(SourceFile::new(name.into(), text.into()));
        (self.files.len() - 1) as u32
    }
    pub fn get(&self, id: u32) -> &SourceFile {
        &self.files[id as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub span: Option<Span>,
    /// Texto breve que se imprime junto al subrayado.
    pub label: Option<String>,
    pub notes: Vec<String>,
}

impl Diagnostic {
    pub fn error(span: Span, message: impl Into<String>) -> Self {
        Diagnostic { severity: Severity::Error, message: message.into(), span: Some(span), label: None, notes: vec![] }
    }
    pub fn warning(span: Span, message: impl Into<String>) -> Self {
        Diagnostic { severity: Severity::Warning, message: message.into(), span: Some(span), label: None, notes: vec![] }
    }
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

pub fn use_color() -> bool {
    std::env::var_os("NO_COLOR").is_none() && std::io::stderr().is_terminal()
}

/// Renderiza un diagnóstico: mensaje, archivo:línea:columna y la línea de código subrayada.
pub fn render(d: &Diagnostic, map: &SourceMap, color: bool) -> String {
    let (red, yellow, blue, bold, reset) =
        if color { ("\x1b[1;31m", "\x1b[1;33m", "\x1b[1;34m", "\x1b[1m", "\x1b[0m") } else { ("", "", "", "", "") };
    let (kind, kc) = match d.severity {
        Severity::Error => ("error", red),
        Severity::Warning => ("warning", yellow),
    };
    let mut out = String::new();
    let _ = writeln!(out, "{kc}{kind}{reset}{bold}: {}{reset}", d.message);
    if let Some(span) = d.span {
        if (span.file as usize) < map.files.len() {
            let f = map.get(span.file);
            let (line, col) = f.line_col(span.lo as usize);
            let gutter = line.to_string().len();
            let pad = " ".repeat(gutter);
            let _ = writeln!(out, "{pad}{blue}-->{reset} {}:{}:{}", f.name, line, col);
            let _ = writeln!(out, "{pad} {blue}|{reset}");
            let text = f.line_text(line);
            let _ = writeln!(out, "{blue}{line:>gutter$} |{reset} {text}");
            let (end_line, end_col) = f.line_col(span.hi as usize);
            let width = if end_line == line { end_col.saturating_sub(col).max(1) } else { text.chars().count().saturating_sub(col - 1).max(1) };
            let mut marker = String::new();
            for ch in text.chars().take(col - 1) {
                marker.push(if ch == '\t' { '\t' } else { ' ' });
            }
            marker.push_str(&"^".repeat(width));
            let label = d.label.as_deref().unwrap_or("");
            let _ = writeln!(out, "{pad} {blue}|{reset} {kc}{marker} {label}{reset}");
            for n in &d.notes {
                let _ = writeln!(out, "{pad} {blue}= nota:{reset} {n}");
            }
            return out;
        }
    }
    for n in &d.notes {
        let _ = writeln!(out, "  {blue}= nota:{reset} {n}");
    }
    out
}

pub fn render_all(ds: &[Diagnostic], map: &SourceMap) -> String {
    let color = use_color();
    ds.iter().map(|d| render(d, map, color)).collect::<Vec<_>>().join("\n")
}
