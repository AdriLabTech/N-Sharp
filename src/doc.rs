//! Generador de documentación HTML (`ncom doc`) a partir de los comentarios `///`.

use crate::ast::*;
use crate::fmt::{fn_signature, generics_str, type_str};
use crate::program::Program;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const CSS: &str = r#"
:root{--bg:#fbfbfd;--fg:#1c1d21;--muted:#6b6f7a;--card:#fff;--line:#e3e5ea;--accent:#5b3fd1;--code:#f1f2f6}
@media (prefers-color-scheme:dark){:root{--bg:#14151a;--fg:#e7e8ec;--muted:#9a9eab;--card:#1c1e25;--line:#2c2f39;--accent:#a38bff;--code:#23252e}}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--fg);font:16px/1.55 system-ui,sans-serif}
main{max-width:860px;margin:0 auto;padding:32px 16px 64px}
h1{font-size:1.9rem;margin:.2em 0}h2{margin-top:2em;border-bottom:1px solid var(--line);padding-bottom:.2em}
a{color:var(--accent);text-decoration:none}a:hover{text-decoration:underline}
.item{background:var(--card);border:1px solid var(--line);border-radius:10px;padding:14px 16px;margin:14px 0}
pre,code{font-family:ui-monospace,Menlo,Consolas,monospace;font-size:.9em}
pre{background:var(--code);padding:10px 12px;border-radius:8px;overflow-x:auto;margin:.4em 0}
code{background:var(--code);padding:1px 5px;border-radius:4px}pre code{background:none;padding:0}
.muted{color:var(--muted)}.members{margin:.5em 0 0 0;padding-left:1.2em}
nav a{margin-right:14px}
"#;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn inline(s: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    for part in s.split('`') {
        if in_code {
            let _ = write!(out, "<code>{}</code>", esc(part));
        } else {
            out.push_str(&esc(part));
        }
        in_code = !in_code;
    }
    out
}

/// Markdown mínimo: párrafos, listas con `-`, bloques ``` y código en línea.
pub fn render_md(doc: &str) -> String {
    let mut out = String::new();
    let mut para: Vec<String> = vec![];
    let mut list = false;
    let mut fence = false;
    let flush = |para: &mut Vec<String>, out: &mut String| {
        if !para.is_empty() {
            let _ = writeln!(out, "<p>{}</p>", inline(&para.join(" ")));
            para.clear();
        }
    };
    for line in doc.lines() {
        if line.trim_start().starts_with("```") {
            flush(&mut para, &mut out);
            if list { out.push_str("</ul>\n"); list = false; }
            out.push_str(if fence { "</code></pre>\n" } else { "<pre><code>" });
            fence = !fence;
            continue;
        }
        if fence {
            let _ = writeln!(out, "{}", esc(line));
            continue;
        }
        if let Some(item) = line.trim_start().strip_prefix("- ") {
            flush(&mut para, &mut out);
            if !list { out.push_str("<ul>\n"); list = true; }
            let _ = writeln!(out, "<li>{}</li>", inline(item));
            continue;
        }
        if list { out.push_str("</ul>\n"); list = false; }
        if line.trim().is_empty() {
            flush(&mut para, &mut out);
        } else {
            para.push(line.trim().to_string());
        }
    }
    flush(&mut para, &mut out);
    if list { out.push_str("</ul>\n"); }
    if fence { out.push_str("</code></pre>\n"); }
    out
}

fn item_block(sig: &str, doc: &Option<String>, extra: &str) -> String {
    let mut s = String::from("<div class=\"item\">\n");
    let _ = writeln!(s, "<pre><code>{}</code></pre>", esc(sig));
    match doc {
        Some(d) => s.push_str(&render_md(d)),
        None => s.push_str("<p class=\"muted\">Sin documentación.</p>\n"),
    }
    s.push_str(extra);
    s.push_str("</div>\n");
    s
}

fn module_page(title: &str, m: &Module, include_private: bool, prefix: &str) -> String {
    let mut body = String::new();
    let mut sections: Vec<(&str, String)> = vec![("Constantes", String::new()), ("Tipos", String::new()), ("Traits", String::new()), ("Funciones", String::new()), ("Implementaciones", String::new())];
    for it in &m.items {
        match &it.kind {
            ItemKind::Const(c) | ItemKind::Static(c) => {
                if c.is_pub || include_private {
                    let kw = if matches!(it.kind, ItemKind::Const(_)) { "const" } else { "static" };
                    sections[0].1.push_str(&item_block(&format!("{}{kw} {} {}", if c.is_pub { "pub " } else { "" }, c.name.name, type_str(&c.ty)), &c.doc, ""));
                }
            }
            ItemKind::Alias(a) => {
                if a.is_pub || include_private {
                    sections[1].1.push_str(&item_block(&format!("type {}{} = {}", a.name.name, generics_str(&a.generics), type_str(&a.ty)), &a.doc, ""));
                }
            }
            ItemKind::Struct(s) => {
                if s.is_pub || include_private {
                    let mut fields = String::new();
                    if !s.fields.is_empty() {
                        fields.push_str("<ul class=\"members\">\n");
                        for f in s.fields.iter().filter(|f| f.is_pub || include_private) {
                            let _ = writeln!(fields, "<li><code>{}{} {}</code></li>", if f.is_pub { "pub " } else { "" }, esc(&f.name.name), esc(&type_str(&f.ty)));
                        }
                        fields.push_str("</ul>\n");
                    }
                    sections[1].1.push_str(&item_block(&format!("{}struct {}{}", if s.is_pub { "pub " } else { "" }, s.name.name, generics_str(&s.generics)), &s.doc, &fields));
                }
            }
            ItemKind::Enum(e) => {
                if e.is_pub || include_private {
                    let mut vs = String::from("<ul class=\"members\">\n");
                    for v in &e.variants {
                        let mut t = v.name.name.clone();
                        if !v.fields.is_empty() {
                            t.push_str(&format!("({})", v.fields.iter().map(type_str).collect::<Vec<_>>().join(", ")));
                        }
                        let _ = writeln!(vs, "<li><code>{}</code></li>", esc(&t));
                    }
                    vs.push_str("</ul>\n");
                    sections[1].1.push_str(&item_block(&format!("{}enum {}{}", if e.is_pub { "pub " } else { "" }, e.name.name, generics_str(&e.generics)), &e.doc, &vs));
                }
            }
            ItemKind::Trait(t) => {
                if t.is_pub || include_private {
                    let mut ms = String::from("<ul class=\"members\">\n");
                    for f in &t.methods {
                        let _ = writeln!(ms, "<li><code>{}</code>{}</li>", esc(&fn_signature(f)), f.doc.as_ref().map(|d| format!(" — {}", inline(d.lines().next().unwrap_or("")))).unwrap_or_default());
                    }
                    ms.push_str("</ul>\n");
                    sections[2].1.push_str(&item_block(&format!("{}trait {}{}", if t.is_pub { "pub " } else { "" }, t.name.name, generics_str(&t.generics)), &t.doc, &ms));
                }
            }
            ItemKind::Fn(f) => {
                if f.is_pub || include_private {
                    sections[3].1.push_str(&item_block(&fn_signature(f), &f.doc, ""));
                }
            }
            ItemKind::Impl(i) => {
                let head = match &i.trait_ {
                    Some(t) => format!("impl{} {} for {}", generics_str(&i.generics), type_str(t), type_str(&i.target)),
                    None => format!("impl{} {}", generics_str(&i.generics), type_str(&i.target)),
                };
                let shown: Vec<&FnDecl> = i.methods.iter().filter(|f| f.is_pub || include_private || i.trait_.is_some()).collect();
                if shown.is_empty() {
                    continue;
                }
                let mut ms = String::new();
                for f in shown {
                    ms.push_str(&item_block(&fn_signature(f), &f.doc, ""));
                }
                sections[4].1.push_str(&format!("<h3><code>{}</code></h3>\n{ms}", esc(&head)));
            }
            _ => {}
        }
    }
    for (name, html) in sections {
        if !html.is_empty() {
            let _ = writeln!(body, "<h2>{name}</h2>\n{html}");
        }
    }
    page(title, prefix, &body)
}

fn page(title: &str, prefix: &str, body: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"es\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n<title>{t}</title><style>{CSS}</style></head>\n<body><main>\n<nav><a href=\"{prefix}index.html\">Índice</a></nav>\n<h1>{t}</h1>\n{body}</main></body></html>\n",
        t = esc(title)
    )
}

/// Genera `index.html` y una página por módulo en `out`. Devuelve los ficheros escritos.
pub fn generate(prog: &Program, project: &str, out: &Path, include_std: bool, include_private: bool) -> Result<Vec<PathBuf>, String> {
    std::fs::create_dir_all(out).map_err(|e| format!("no se pudo crear {}: {e}", out.display()))?;
    let mut written = vec![];
    let mut index = String::new();
    let _ = writeln!(index, "<p class=\"muted\">Documentación generada por <code>ncom doc</code>.</p>\n<h2>Módulos</h2>\n<ul>");
    for (i, m) in prog.modules.iter().enumerate() {
        if i == 0 || (m.is_std && !include_std) {
            continue;
        }
        let display = if m.name == "main" { project.to_string() } else { m.name.clone() };
        let file = format!("{}.html", m.name.replace('.', "_"));
        let html = module_page(&format!("Módulo {display}"), &m.ast, include_private, "");
        let path = out.join(&file);
        std::fs::write(&path, html).map_err(|e| format!("{}: {e}", path.display()))?;
        written.push(path);
        let _ = writeln!(index, "<li><a href=\"{file}\">{}</a>{}</li>", esc(&display), if m.is_std { " <span class=\"muted\">(estándar)</span>" } else { "" });
    }
    index.push_str("</ul>\n");
    let ip = out.join("index.html");
    std::fs::write(&ip, page(&format!("{project} — documentación"), "", &index)).map_err(|e| format!("{}: {e}", ip.display()))?;
    written.insert(0, ip);
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown() {
        let h = render_md("Suma `a` y `b`.\n\n- uno\n- dos\n\n```\ncódigo <x>\n```");
        assert!(h.contains("<code>a</code>"));
        assert!(h.contains("<li>uno</li>"));
        assert!(h.contains("&lt;x&gt;"));
    }
}
