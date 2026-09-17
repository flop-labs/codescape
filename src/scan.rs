//! Repository scan: list tracked source files and classify every character
//! into a small set of syntax classes used for colouring.

use std::path::Path;
use std::process::Command;

pub const C_SPACE: u8 = 0;
pub const C_IDENT: u8 = 1;
pub const C_KEYWORD: u8 = 2;
pub const C_TYPE: u8 = 3;
pub const C_STRING: u8 = 4;
pub const C_NUMBER: u8 = 5;
pub const C_COMMENT: u8 = 6;
pub const C_PUNCT: u8 = 7;
pub const C_HEADING: u8 = 8;
pub const C_CALL: u8 = 9;
pub const CLASS_COUNT: usize = 10;

/// Glyph index used for characters outside printable ASCII.
pub const GLYPH_OTHER: u8 = 127;

const EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "mjs", "cjs", "py", "jl", "qnt", "lean", "sol", "go", "sh", "toml",
    "md", "yml", "yaml", "css", "json", "tla", "just",
];
const MAX_LINES: usize = 20_000;
const MAX_AVG_LINE: usize = 240;

#[derive(Clone, Copy, PartialEq)]
pub enum Lang {
    CLike,
    Rust,
    Hash,
    Lean,
    Markdown,
    Json,
}

pub struct SourceFile {
    pub path: String,
    /// One byte per character: printable ASCII, or `GLYPH_OTHER`.
    pub text: Vec<u8>,
    pub class: Vec<u8>,
    /// Byte offset of each line start in `text`; lines exclude the newline.
    pub line_starts: Vec<u32>,
}

impl SourceFile {
    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    pub fn line(&self, i: usize) -> (&[u8], &[u8]) {
        let s = self.line_starts[i] as usize;
        let e = if i + 1 < self.line_starts.len() {
            self.line_starts[i + 1] as usize - 1
        } else {
            self.text.len()
        };
        (&self.text[s..e], &self.class[s..e])
    }
}

pub fn scan_repo(root: &Path) -> Vec<SourceFile> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z"])
        .output()
        .expect("git ls-files failed; codescape needs a git checkout");
    let mut files = Vec::new();
    for rel in out.stdout.split(|b| *b == 0) {
        let Ok(rel) = std::str::from_utf8(rel) else {
            continue;
        };
        if rel.is_empty() || skip_path(rel) {
            continue;
        }
        let ext = rel.rsplit('.').next().unwrap_or("");
        let base = rel.rsplit('/').next().unwrap_or(rel);
        if !EXTENSIONS.contains(&ext) && base != "justfile" && base != "Dockerfile" {
            continue;
        }
        let Ok(bytes) = std::fs::read(root.join(rel)) else {
            continue;
        };
        if bytes.contains(&0) {
            continue;
        }
        let src = String::from_utf8_lossy(&bytes);
        let lines = src.lines().count();
        if lines == 0 || lines > MAX_LINES || src.len() / lines > MAX_AVG_LINE {
            continue;
        }
        let lang = match ext {
            "rs" => Lang::Rust,
            "py" | "sh" | "toml" | "yml" | "yaml" | "jl" | "just" => Lang::Hash,
            "lean" => Lang::Lean,
            "md" => Lang::Markdown,
            "json" => Lang::Json,
            _ if base == "justfile" || base == "Dockerfile" => Lang::Hash,
            _ => Lang::CLike,
        };
        files.push(classify(rel.to_string(), &src, lang));
    }
    files
}

fn skip_path(rel: &str) -> bool {
    rel.contains("node_modules/")
        || rel.contains("/vendor/")
        || rel.ends_with(".lock")
        || rel.ends_with("-lock.yaml")
        || rel.ends_with("lock.json")
        || rel.contains(".min.")
        || rel.contains(".bundle.")
        || rel.ends_with("Manifest.toml")
}

fn map_char(c: char) -> u8 {
    match c {
        ' '..='~' => c as u8,
        '\u{2014}' | '\u{2013}' | '\u{2212}' => b'-',
        '\u{2192}' | '\u{21d2}' => b'>',
        '\u{2190}' => b'<',
        '\u{2018}' | '\u{2019}' => b'\'',
        '\u{201c}' | '\u{201d}' => b'"',
        '\u{2264}' => b'<',
        '\u{2265}' => b'>',
        '\u{00d7}' => b'x',
        _ => GLYPH_OTHER,
    }
}

const KEYWORDS: &[&str] = &[
    "as",
    "async",
    "await",
    "break",
    "const",
    "continue",
    "crate",
    "dyn",
    "else",
    "enum",
    "extern",
    "false",
    "fn",
    "for",
    "if",
    "impl",
    "in",
    "let",
    "loop",
    "match",
    "mod",
    "move",
    "mut",
    "pub",
    "ref",
    "return",
    "self",
    "Self",
    "static",
    "struct",
    "super",
    "trait",
    "true",
    "type",
    "unsafe",
    "use",
    "where",
    "while",
    "import",
    "export",
    "from",
    "function",
    "class",
    "interface",
    "extends",
    "implements",
    "new",
    "this",
    "var",
    "null",
    "undefined",
    "void",
    "typeof",
    "instanceof",
    "try",
    "catch",
    "finally",
    "throw",
    "switch",
    "case",
    "default",
    "def",
    "lambda",
    "None",
    "True",
    "False",
    "and",
    "or",
    "not",
    "is",
    "with",
    "yield",
    "pass",
    "raise",
    "except",
    "elif",
    "del",
    "global",
    "nonlocal",
    "readonly",
    "private",
    "public",
    "protected",
    "declare",
    "namespace",
    "keyof",
    "module",
    "theorem",
    "lemma",
    "structure",
    "inductive",
    "namespace",
    "end",
    "open",
    "then",
    "do",
    "val",
    "pure",
    "action",
    "temporal",
    "all",
    "any",
    "nondet",
    "contract",
    "require",
    "emit",
    "mapping",
    "returns",
    "external",
    "view",
    "payable",
    "local",
    "echo",
    "fi",
    "done",
    "esac",
];

fn classify(path: String, src: &str, lang: Lang) -> SourceFile {
    let mut text = Vec::with_capacity(src.len());
    let mut line_starts = vec![0u32];
    for (i, line) in src.lines().enumerate() {
        if i > 0 {
            text.push(b'\n');
            line_starts.push(text.len() as u32);
        }
        for c in line.chars() {
            if c == '\t' {
                text.extend_from_slice(b"    ");
            } else {
                text.push(map_char(c));
            }
        }
    }
    let class = lex(&text, lang);
    SourceFile {
        path,
        text,
        class,
        line_starts,
    }
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c == b'$'
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$'
}

fn lex(t: &[u8], lang: Lang) -> Vec<u8> {
    let n = t.len();
    let mut cls = vec![C_SPACE; n];
    if lang == Lang::Markdown {
        lex_markdown(t, &mut cls);
        return cls;
    }
    let mut i = 0;
    let at = |i: usize, s: &[u8]| t[i..].starts_with(s);
    while i < n {
        let c = t[i];
        if c == b'\n' || c == b' ' {
            i += 1;
            continue;
        }
        let line_comment = match lang {
            Lang::Hash => c == b'#',
            Lang::Lean => at(i, b"--"),
            Lang::Json => false,
            _ => at(i, b"//"),
        };
        if line_comment {
            while i < n && t[i] != b'\n' {
                cls[i] = C_COMMENT;
                i += 1;
            }
            continue;
        }
        let block = match lang {
            Lang::CLike | Lang::Rust => at(i, b"/*").then_some(&b"*/"[..]),
            Lang::Lean => at(i, b"/-").then_some(&b"-/"[..]),
            _ => None,
        };
        if let Some(close) = block {
            let start = i;
            i += 2;
            while i < n && !at(i, close) {
                i += 1;
            }
            i = (i + 2).min(n);
            for k in start..i {
                if t[k] != b'\n' && t[k] != b' ' {
                    cls[k] = C_COMMENT;
                }
            }
            continue;
        }
        if lang == Lang::Hash && (at(i, b"\"\"\"") || at(i, b"'''")) {
            let q = &t[i..i + 3];
            let start = i;
            i += 3;
            while i < n && !at(i, q) {
                i += 1;
            }
            i = (i + 3).min(n);
            mark_string(t, &mut cls, start, i);
            continue;
        }
        let quote = c == b'"'
            || c == b'`'
            || (c == b'\'' && lang != Lang::Rust && lang != Lang::Lean)
            || (c == b'\''
                && lang == Lang::Rust
                && i + 2 < n
                && (t[i + 2] == b'\'' || t[i + 1] == b'\\'));
        if quote {
            let start = i;
            i += 1;
            while i < n && t[i] != c {
                if t[i] == b'\\' {
                    i += 1;
                }
                if i < n && t[i] == b'\n' && c != b'`' && lang != Lang::Rust {
                    break;
                }
                i += 1;
            }
            i = (i + 1).min(n);
            mark_string(t, &mut cls, start, i);
            continue;
        }
        if c.is_ascii_digit() {
            while i < n && (t[i].is_ascii_alphanumeric() || t[i] == b'_' || t[i] == b'.') {
                cls[i] = C_NUMBER;
                i += 1;
            }
            continue;
        }
        if is_ident_start(c) {
            let start = i;
            while i < n && is_ident(t[i]) {
                i += 1;
            }
            let word = std::str::from_utf8(&t[start..i]).unwrap_or("");
            let k = if lang == Lang::Json {
                if start > 0 && t[start - 1] == b'"' {
                    C_STRING
                } else {
                    C_KEYWORD
                }
            } else if KEYWORDS.contains(&word) {
                C_KEYWORD
            } else if i < n && (t[i] == b'(' || (t[i] == b'!' && lang == Lang::Rust)) {
                C_CALL
            } else if c.is_ascii_uppercase() {
                C_TYPE
            } else {
                C_IDENT
            };
            cls[start..i].fill(k);
            continue;
        }
        cls[i] = if c == b'#' && lang == Lang::Rust {
            C_HEADING
        } else {
            C_PUNCT
        };
        i += 1;
    }
    cls
}

fn mark_string(t: &[u8], cls: &mut [u8], s: usize, e: usize) {
    for k in s..e {
        if t[k] != b'\n' && t[k] != b' ' {
            cls[k] = C_STRING;
        }
    }
}

fn lex_markdown(t: &[u8], cls: &mut [u8]) {
    let mut in_fence = false;
    let mut s = 0;
    while s <= t.len() {
        let e = t[s..]
            .iter()
            .position(|b| *b == b'\n')
            .map(|p| s + p)
            .unwrap_or(t.len());
        let line = &t[s..e];
        let trimmed = line
            .iter()
            .position(|b| *b != b' ')
            .map(|p| &line[p..])
            .unwrap_or(&[]);
        let fence = trimmed.starts_with(b"```");
        let k = if fence || in_fence {
            C_STRING
        } else if trimmed.starts_with(b"#") {
            C_HEADING
        } else if trimmed.starts_with(b">") {
            C_COMMENT
        } else if trimmed.starts_with(b"|") {
            C_PUNCT
        } else {
            C_IDENT
        };
        if fence {
            in_fence = !in_fence;
        }
        let mut in_code = false;
        for (j, b) in line.iter().enumerate() {
            if *b == b' ' {
                continue;
            }
            if k == C_IDENT && *b == b'`' {
                in_code = !in_code;
                cls[s + j] = C_PUNCT;
                continue;
            }
            cls[s + j] = if in_code {
                C_TYPE
            } else if k == C_IDENT && (*b == b'*' || *b == b'[' || *b == b']' || *b == b'-') {
                C_PUNCT
            } else {
                k
            };
        }
        s = e + 1;
    }
}
