//! Policy checks (donor `tools/check-policy.ts`).
//!
//! The donor drives the TypeScript compiler API: most of its rules query the
//! type checker (`hasAny`, `erasesUnsafeValue`, symbol resolution) and expand
//! the program through full module resolution. This port checks the same
//! project with the shared [`ts_scan`](crate::inventory::ts_scan) tokenizer
//! instead, so every rule below is a syntactic approximation of the donor
//! rule with the same name:
//!
//! * File discovery, native-header sniffing, symlink and
//!   implementation-language rules, the `src/types/png.d.ts` exception, and
//!   the tsconfig compiler-policy matrix are exact ports.
//! * `suppression`, `explicit-any`, `assertion` (`as`, postfix `!`, `!:`,
//!   `asserts`), `non-null`, `definite-assignment`, `ambient` (`declare`),
//!   `source-boundary`, `runtime-boundary`, `native-implementation`,
//!   `ffi-boundary`, `ffi-binding`, and `runtime-subprocess` (loader imports)
//!   are token-level approximations: an identifier named `any`, `as`, or
//!   `declare` in value position can report where the donor stays silent,
//!   angle-bracket assertions are not recognized, and bare module
//!   specifiers are assumed to resolve into `node_modules` (the donor
//!   tsconfig declares no `paths` mappings).
//! * `unsafe-any`, `unsafe-conversion`, `unsafe-assignment`,
//!   `unsafe-argument`, `unsafe-return`, `async-void`,
//!   `dynamic-implementation`, `native-loader`, `ffi-library`, `ffi-symbol`,
//!   `ffi-pointer`, and `typescript` (emitted compiler diagnostics) need the
//!   TypeScript type checker and have no syntactic equivalent here; the
//!   donor toolchain (`bun run policy`, also driven by
//!   [`crate::build`] gates) remains the enforcer for those rules.
//!
//! Diagnostic order matches the donor: sorted per-file diagnostics, then
//! compiler-policy entries, then file-level entries.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::error::ToolsError;
use crate::fsutil::{lexical_absolute, posix_relative};
use crate::inventory::ts_scan::{self, TokKind, Token};
use crate::js::{compare_text, line_col_utf16, line_starts};
use crate::json::{parse_jsonc, Json};

/// Policy violation (donor `PolicyDiagnostic`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyDiagnostic {
    /// Absolute file path.
    pub file: String,
    /// 1-based line.
    pub line: usize,
    /// 1-based column in UTF-16 code units.
    pub column: usize,
    /// Rule name.
    pub rule: &'static str,
    /// Human-readable message.
    pub message: String,
}

/// Donor diagnostic order: file, line, column, rule.
pub fn sort_diagnostics(diagnostics: &mut [PolicyDiagnostic]) {
    diagnostics.sort_by(|left, right| {
        compare_text(&left.file, &right.file)
            .then(left.line.cmp(&right.line))
            .then(left.column.cmp(&right.column))
            .then(compare_text(left.rule, right.rule))
    });
}

/// Render one diagnostic the way the donor CLI prints it.
#[must_use]
pub fn render_diagnostic(root: &Path, diagnostic: &PolicyDiagnostic) -> String {
    format!(
        "{}:{}:{} [{}] {}",
        posix_relative(root, Path::new(&diagnostic.file)),
        diagnostic.line,
        diagnostic.column,
        diagnostic.rule,
        diagnostic.message
    )
}

/// Whether a module name is a native artifact (donor `nativeArtifactName`).
fn is_native_artifact_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    for suffix in [".node", ".dll", ".dylib", ".a", ".o", ".obj", ".lib", ".exe"] {
        if lower.ends_with(suffix) {
            return true;
        }
    }
    let mut rest = lower.as_str();
    while let Some(index) = rest.find(".so") {
        let mut tail = &rest[index + 3..];
        let mut valid = true;
        while let Some(stripped) = tail.strip_prefix('.') {
            let digits = stripped.bytes().take_while(u8::is_ascii_digit).count();
            if digits == 0 {
                valid = false;
                break;
            }
            tail = &stripped[digits..];
        }
        if valid && tail.is_empty() {
            return true;
        }
        rest = &rest[index + 1..];
    }
    false
}

/// Implementation extensions that join discovery so they can be rejected.
fn has_implementation_extension(name: &str) -> bool {
    let Some(dot) = name.rfind('.') else {
        return false;
    };
    matches!(
        name[dot..].to_ascii_lowercase().as_str(),
        ".js"
            | ".jsx"
            | ".mjs"
            | ".cjs"
            | ".mts"
            | ".cts"
            | ".tsx"
            | ".c"
            | ".h"
            | ".cc"
            | ".cpp"
            | ".cxx"
            | ".hpp"
            | ".m"
            | ".mm"
            | ".rs"
            | ".go"
            | ".py"
            | ".wasm"
            | ".wat"
            | ".glsl"
            | ".vert"
            | ".frag"
    )
}

/// Whether a file starts with a native magic header (donor `hasNativeHeader`).
#[must_use]
pub fn has_native_header(path: &Path) -> bool {
    use std::io::Read as _;
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut header = [0u8; 8];
    let Ok(count) = file.read(&mut header) else {
        return false;
    };
    if count < 4 {
        return false;
    }
    let magic = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
    if magic == 0x7f45_4c46 || u16::from_be_bytes([header[0], header[1]]) == 0x4d5a {
        return true;
    }
    if matches!(
        magic,
        0x0061_736d
            | 0xfeed_face
            | 0xfeed_facf
            | 0xcefa_edfe
            | 0xcffa_edfe
            | 0xcafe_babe
            | 0xbeba_feca
            | 0xcafe_babf
            | 0xbfba_feca
    ) {
        return true;
    }
    count == 8 && header == *b"!<arch>\n"
}

/// Discover audited files (donor `discoverFiles`).
pub fn discover_files(root: &Path) -> Result<Vec<PathBuf>, ToolsError> {
    fn visit(directory: &Path, root: &Path, files: &mut Vec<PathBuf>) -> Result<(), ToolsError> {
        let entries = std::fs::read_dir(directory)
            .map_err(|error| ToolsError::io(format!("listing {}", directory.display()), error))?;
        for entry in entries {
            let entry = entry.map_err(|error| ToolsError::io(format!("listing {}", directory.display()), error))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if directory == root && (name == "node_modules" || name == ".git" || name == "dist" || name == ".artifacts")
            {
                continue;
            }
            let file_type = entry
                .file_type()
                .map_err(|error| ToolsError::io(format!("stating {}", entry.path().display()), error))?;
            let path = directory.join(&name);
            if file_type.is_dir() {
                visit(&path, root, files)?;
            } else if file_type.is_symlink()
                || (file_type.is_file()
                    && (name.ends_with(".ts")
                        || has_implementation_extension(&name)
                        || is_native_artifact_name(&name)
                        || has_native_header(&path)))
            {
                files.push(path);
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    visit(root, root, &mut files)?;
    files.sort_by(|left, right| compare_text(&left.to_string_lossy(), &right.to_string_lossy()));
    Ok(files)
}

/// Exact `src/types/png.d.ts` body exempt from the ambient rules.
const PNG_DECLARATION: &str = "declare module \"*.png\" {\n  const path: string;\n  export default path;\n}";

/// Menu artwork imports allowed through the runtime boundary with a file attribute.
const MENU_ART: [&str; 4] = [
    "assets/ui/menu-background.png",
    "assets/ui/menu-panel.png",
    "assets/ui/menu-focus.png",
    "assets/ui/main-menu-background.png",
];

/// Per-file audit state; `seen` is the donor's per-file `(position, rule)` guard.
struct FileAudit<'a> {
    root: &'a Path,
    file: String,
    project_path: String,
    runtime: bool,
    platform: bool,
    source: &'a str,
    starts: Vec<usize>,
    seen: HashSet<(usize, &'static str)>,
    out: Vec<PolicyDiagnostic>,
}

impl<'a> FileAudit<'a> {
    fn report(&mut self, offset: usize, rule: &'static str, message: &str) {
        if !self.seen.insert((offset, rule)) {
            return;
        }
        let (line, column) = line_col_utf16(self.source, &self.starts, offset.min(self.source.len()));
        self.out.push(PolicyDiagnostic {
            file: self.file.clone(),
            line,
            column,
            rule,
            message: message.to_owned(),
        });
    }

    fn module_boundary(&mut self, module: &str, offset: usize, file_attribute: bool) {
        let clean = module.split(['?', '#']).next().unwrap_or(module);
        if is_native_artifact_name(clean) {
            self.report(
                offset,
                "native-implementation",
                "Native addons and libraries cannot be project modules; use the SDL2/OpenGL platform boundary.",
            );
        }
        if module == "bun:ffi" && !self.platform {
            self.report(offset, "ffi-boundary", "Only src/platform modules may import bun:ffi.");
        }
        let builtin = module.strip_prefix("node:").unwrap_or(module);
        if self.runtime && matches!(builtin, "child_process" | "cluster" | "module") {
            self.report(
                offset,
                "runtime-subprocess",
                "Runtime modules cannot load subprocess or CommonJS loader APIs; subprocess orchestration belongs in tools/tests.",
            );
        }
        let local = if module.starts_with('.') {
            let directory = Path::new(&self.file).parent().unwrap_or(self.root);
            Some(lexical_absolute(directory, module))
        } else if Path::new(module).is_absolute() {
            Some(PathBuf::from(module))
        } else {
            None
        };
        if let Some(local) = local {
            let imported = posix_relative(self.root, &local);
            if imported.starts_with("../")
                || imported.starts_with("dist/")
                || imported.starts_with(".artifacts/")
                || imported.starts_with(".git/")
            {
                self.report(
                    offset,
                    "source-boundary",
                    "Project modules cannot import excluded build output or external source paths.",
                );
            }
            let menu = self.project_path == "src/app/bootstrap/menu-art.ts"
                && MENU_ART.contains(&imported.as_str())
                && file_attribute;
            if self.runtime && !imported.starts_with("src/") && !imported.starts_with("node_modules/") && !menu {
                self.report(
                    offset,
                    "runtime-boundary",
                    "Local runtime imports must stay within src; build tools, tests, and generated artifacts cannot implement runtime behavior.",
                );
            }
        }
    }
}

/// Token text.
fn text<'a>(source: &'a str, token: &Token) -> &'a str {
    &source[token.start..token.end.min(source.len())]
}

/// Unescape a single- or double-quoted TypeScript string literal.
fn unescape_string(raw: &str) -> String {
    if raw.len() < 2 {
        return raw.to_owned();
    }
    let body = &raw[1..raw.len().saturating_sub(1)];
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('b') => out.push('\u{8}'),
            Some('f') => out.push('\u{c}'),
            Some('v') => out.push('\u{b}'),
            Some('0') => out.push('\0'),
            Some('x') => {
                let hex: String = chars.by_ref().take(2).collect();
                out.push(
                    u32::from_str_radix(&hex, 16)
                        .ok()
                        .and_then(char::from_u32)
                        .unwrap_or('\u{fffd}'),
                );
            }
            Some('u') => {
                let rest: String = chars.clone().collect();
                if let Some(braced) = rest.strip_prefix('{') {
                    let hex: String = braced.chars().take_while(|ch| *ch != '}').collect();
                    for _ in 0..hex.len() + 2 {
                        chars.next();
                    }
                    out.push(
                        u32::from_str_radix(&hex, 16)
                            .ok()
                            .and_then(char::from_u32)
                            .unwrap_or('\u{fffd}'),
                    );
                } else {
                    let hex: String = chars.by_ref().take(4).collect();
                    out.push(
                        u32::from_str_radix(&hex, 16)
                            .ok()
                            .and_then(char::from_u32)
                            .unwrap_or('\u{fffd}'),
                    );
                }
            }
            Some('\r') => {
                if chars.clone().next() == Some('\n') {
                    chars.next();
                }
            }
            Some('\n' | '\u{2028}' | '\u{2029}') => {}
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// Whether a suppression marker occurs at a word boundary (donor `suppression` pattern).
fn has_suppression(comment: &str) -> bool {
    const MARKERS: [&str; 3] = ["@ts-ignore", "@ts-expect-error", "@ts-nocheck"];
    let mut rest = comment;
    while let Some(index) = rest.find("eslint-disable") {
        let after = &rest[index + "eslint-disable".len()..];
        let end = if let Some(tail) = after.strip_prefix("-next-line") {
            tail
        } else if let Some(tail) = after.strip_prefix("-line") {
            tail
        } else {
            after
        };
        if end
            .chars()
            .next()
            .is_none_or(|ch| !(ch.is_alphanumeric() || ch == '_' || ch == '$'))
        {
            return true;
        }
        rest = &rest[index + 1..];
    }
    for marker in MARKERS {
        let mut tail = comment;
        while let Some(index) = tail.find(marker) {
            let after = &tail[index + marker.len()..];
            if after
                .chars()
                .next()
                .is_none_or(|ch| !(ch.is_alphanumeric() || ch == '_' || ch == '$'))
            {
                return true;
            }
            tail = &tail[index + 1..];
        }
    }
    false
}

/// Brace ranges owned by import/export named bindings, so `as`/`any` inside
/// `import {a as b}` / `export {a as b}` are not mistaken for assertions.
fn binding_braces(source: &str, tokens: &[Token]) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if token.kind != TokKind::Ident || !matches!(text(source, token), "import" | "export") {
            continue;
        }
        if index > 0 && matches!(text(source, &tokens[index - 1]), "." | "?.") {
            continue;
        }
        let mut cursor = index + 1;
        if cursor < tokens.len() && text(source, &tokens[cursor]) == "type" {
            let after = cursor + 1;
            if after < tokens.len() && text(source, &tokens[after]) != "from" {
                cursor = after;
            }
        }
        if cursor < tokens.len() && text(source, &tokens[cursor]) == "{" {
            let mut depth: usize = 0;
            for (close, candidate) in tokens.iter().enumerate().skip(cursor) {
                match text(source, candidate) {
                    "{" => depth += 1,
                    "}" => {
                        depth -= 1;
                        if depth == 0 {
                            ranges.push((cursor, close));
                            break;
                        }
                    }
                    ";" if depth == 0 => break,
                    _ => {}
                }
            }
        }
    }
    ranges
}

/// Whether token `index` sits inside an import/export binding brace range.
fn in_bindings(ranges: &[(usize, usize)], index: usize) -> bool {
    ranges.iter().any(|(start, end)| *start <= index && index <= *end)
}

/// Identifiers that can never end a value expression, so a following `!` is a
/// prefix negation rather than a postfix assertion (`return !flag`).
const NON_VALUE_WORDS: [&str; 39] = [
    "return",
    "typeof",
    "case",
    "in",
    "of",
    "new",
    "delete",
    "void",
    "instanceof",
    "yield",
    "await",
    "throw",
    "else",
    "do",
    "if",
    "for",
    "while",
    "switch",
    "with",
    "export",
    "import",
    "from",
    "as",
    "is",
    "extends",
    "implements",
    "keyof",
    "infer",
    "type",
    "interface",
    "class",
    "function",
    "enum",
    "namespace",
    "module",
    "satisfies",
    "let",
    "const",
    "var",
];

/// Value-only operators that may precede a variable but never a type
/// (`&& any` reads a variable; `&` alone can join types and still flags).
const VALUE_ONLY_PREV: [&str; 25] = [
    "&&", "||", "??", "===", "==", "!==", "!=", "<=", ">=", ">", "++", "--", "**", "<<", ">>", ">>>", "...", "!", "+",
    "-", "*", "/", "%", "^", "~",
];

/// Value-only operators that may follow a variable but never a type
/// (generic closers are excluded: `Map<K, any>` must still flag).
const VALUE_ONLY_NEXT: [&str; 22] = [
    "&&", "||", "??", "===", "==", "!==", "!=", "<=", ">=", "++", "--", "**", "<<", "...", "!", "+", "-", "*", "/",
    "%", "^", "~",
];

/// Whether the token before `index` (skipping a `!` run) ends a value.
fn postfix_bang_target(source: &str, tokens: &[Token], index: usize) -> bool {
    let mut cursor = index;
    while cursor > 0 && text(source, &tokens[cursor - 1]) == "!" {
        cursor -= 1;
    }
    if cursor == 0 {
        return false;
    }
    let previous = &tokens[cursor - 1];
    if matches!(text(source, previous), ")" | "]") {
        return true;
    }
    match previous.kind {
        TokKind::Ident => !NON_VALUE_WORDS.contains(&text(source, previous)),
        TokKind::Str | TokKind::Number | TokKind::TmplTail | TokKind::TmplNoSub => true,
        _ => false,
    }
}

/// Audit one TypeScript source (donor `auditProgram` per-file pass, syntactic subset).
pub fn audit_source(project_root: &Path, file: &Path, source_text: &str) -> Vec<PolicyDiagnostic> {
    let file_string = file.to_string_lossy().into_owned();
    let project_path = posix_relative(project_root, file);
    let mut audit = FileAudit {
        root: project_root,
        file: file_string,
        project_path: project_path.clone(),
        runtime: project_path.starts_with("src/"),
        platform: project_path.starts_with("src/platform/"),
        source: source_text,
        starts: line_starts(source_text),
        seen: HashSet::new(),
        out: Vec::new(),
    };
    let png = project_path == "src/types/png.d.ts"
        && crate::js::trim_js(&source_text.replace("\r\n", "\n")) == PNG_DECLARATION;
    if file.to_string_lossy().ends_with(".d.ts") && !png {
        audit.report(
            0,
            "ambient",
            "Project declaration files can hide implementation; use checked TypeScript definitions.",
        );
    }
    let scan = ts_scan::tokenize(source_text);
    for comment in &scan.comments {
        if has_suppression(&comment.text) {
            audit.report(comment.start, "suppression", "Diagnostic suppression is forbidden.");
        }
        if let Some(reference) = reference_path(&comment.text) {
            let directory = file.parent().unwrap_or(project_root);
            let resolved = lexical_absolute(directory, &reference);
            audit.module_boundary(&resolved.to_string_lossy(), 0, false);
        }
    }
    let tokens = scan.tokens;
    let braces = binding_braces(source_text, &tokens);
    for (index, token) in tokens.iter().enumerate() {
        let word = text(source_text, token);
        let previous = if index > 0 {
            Some(text(source_text, &tokens[index - 1]))
        } else {
            None
        };
        let next = if index + 1 < tokens.len() {
            Some(text(source_text, &tokens[index + 1]))
        } else {
            None
        };
        match token.kind {
            TokKind::Ident => match word {
                "declare" => {
                    let member = matches!(previous, Some("." | "?.")) || previous == Some("function");
                    let value_use = matches!(next, Some("(" | "=" | ":" | "=>"));
                    if !png && !member && !value_use {
                        // The donor reports the declared statement; starting at a
                        // leading `export` lets declaration files dedup against
                        // their file-level diagnostic exactly like the donor.
                        let mut start = token.start;
                        if index > 0 && matches!(text(source_text, &tokens[index - 1]), "export" | "default") {
                            start = tokens[index - 1].start;
                        }
                        audit.report(start, "ambient", "Project ambient declarations are forbidden.");
                    }
                }
                "any" => {
                    let member = matches!(previous, Some("." | "?."));
                    let declaration = matches!(
                        previous,
                        Some("function" | "class" | "interface" | "type" | "import" | "export")
                    );
                    let alias = previous == Some("=")
                        && index >= 3
                        && tokens[index - 2].kind == TokKind::Ident
                        && text(source_text, &tokens[index - 3]) == "type";
                    let value_use = matches!(next, Some(":" | "("))
                        || next == Some("=") && previous != Some(":")
                        || previous == Some("=") && !alias
                        || previous == Some("(") && matches!(next, Some(")" | "," | "=>"))
                        || matches!(previous, Some("{" | ",")) && next == Some("}")
                        || previous.is_some_and(|word| VALUE_ONLY_PREV.contains(&word))
                        || next.is_some_and(|word| VALUE_ONLY_NEXT.contains(&word))
                        || next == Some("?") && previous != Some("extends");
                    if !member && !declaration && !value_use && !in_bindings(&braces, index) {
                        audit.report(
                            token.start,
                            "explicit-any",
                            "Explicit any is forbidden; narrow unknown at the boundary.",
                        );
                    }
                }
                "as" => {
                    let non_value = matches!(
                        previous,
                        None | Some("(" | "[" | "{" | "," | ";" | ":" | "=" | "=>" | "?" | "*" | "." | "?.")
                    );
                    let non_type = matches!(next, None | Some("," | ";" | "=" | "(" | ")" | "]" | "}" | ":"));
                    let assertion = !non_value && !non_type && !in_bindings(&braces, index);
                    if assertion {
                        audit.report(
                            token.start,
                            "assertion",
                            "Type assertions, including const assertions, are forbidden.",
                        );
                    }
                }
                "asserts" => {
                    if tokens.get(index + 1).is_some_and(|after| after.kind == TokKind::Ident) {
                        audit.report(
                            token.start,
                            "assertion",
                            "Assertion functions cannot bypass checked boundary narrowing.",
                        );
                    }
                }
                "import" => {
                    if !matches!(previous, Some("." | "?.")) {
                        audit_import(&mut audit, source_text, &tokens, index);
                    }
                }
                "export" => {
                    if !matches!(previous, Some("." | "?.")) {
                        audit_export(&mut audit, source_text, &tokens, index);
                    }
                }
                _ => {}
            },
            TokKind::Punct if word == "!" => {
                if postfix_bang_target(source_text, &tokens, index) {
                    if next == Some(":") {
                        audit.report(
                            token.start,
                            "definite-assignment",
                            "Definite-assignment assertions are forbidden; initialize owned state.",
                        );
                    } else {
                        audit.report(
                            token.start,
                            "non-null",
                            "Non-null assertions are forbidden; check the value.",
                        );
                    }
                }
            }
            _ => {}
        }
    }
    audit.out
}

/// Extract a `/// <reference path="..." />` target from a comment.
fn reference_path(comment: &str) -> Option<String> {
    let marker = comment.find("<reference")?;
    let rest = &comment[marker..];
    let key = rest.find("path")?;
    let after = rest[key + "path".len()..].trim_start();
    let value = after.strip_prefix('=')?.trim_start();
    let quote = value.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    Some(value[1..].split(quote).next().unwrap_or_default().to_owned())
}

/// Audit one `import` statement or dynamic import.
fn audit_import(audit: &mut FileAudit<'_>, source: &str, tokens: &[Token], index: usize) {
    let offset = tokens[index].start;
    let next = tokens.get(index + 1);
    if next.is_some_and(|token| text(source, token) == "(") {
        let argument = tokens.get(index + 2);
        match argument {
            Some(token) if token.kind == TokKind::Str => {
                let module = unescape_string(text(source, token));
                audit.module_boundary(&module, offset, false);
                if module == "bun:ffi" {
                    audit.report(
                        offset,
                        "ffi-binding",
                        "Use named bun:ffi imports so platform loader calls remain statically visible.",
                    );
                }
            }
            _ => {
                if audit.runtime {
                    audit.report(
                        offset,
                        "runtime-boundary",
                        "Runtime dynamic imports require statically known module names so native and tool boundaries can be checked.",
                    );
                }
            }
        }
        return;
    }
    if next.is_some_and(|token| token.kind == TokKind::Str) {
        let module = unescape_string(text(source, &tokens[index + 1]));
        audit.module_boundary(&module, offset, false);
        if module == "bun:ffi" {
            audit.report(
                offset,
                "ffi-binding",
                "Use named bun:ffi imports so platform loader calls remain statically visible.",
            );
        }
        return;
    }
    let mut cursor = index + 1;
    if cursor < tokens.len() && text(source, &tokens[cursor]) == "type" {
        let after = cursor + 1;
        if after < tokens.len() && text(source, &tokens[after]) != "from" {
            cursor = after;
        }
    }
    if cursor + 1 < tokens.len() && tokens[cursor].kind == TokKind::Ident && text(source, &tokens[cursor + 1]) == "=" {
        for (step, token) in tokens.iter().enumerate().skip(cursor + 2) {
            let word = text(source, token);
            if word == ";" {
                break;
            }
            if token.kind == TokKind::Str && step > cursor + 2 && text(source, &tokens[step - 1]) == "(" {
                let module = unescape_string(word);
                audit.module_boundary(&module, offset, false);
                if module == "bun:ffi" {
                    audit.report(
                        offset,
                        "ffi-binding",
                        "Use named bun:ffi imports so platform loader calls remain statically visible.",
                    );
                }
                break;
            }
        }
        return;
    }
    let bindings_start = cursor;
    let mut depth: usize = 0;
    let mut cursor = cursor;
    while cursor < tokens.len() {
        let word = text(source, &tokens[cursor]);
        if word == "{" {
            depth += 1;
        } else if word == "}" {
            depth = depth.saturating_sub(1);
        } else if word == ";" {
            break;
        } else if word == "from" && depth == 0 {
            let specifier = tokens.get(cursor + 1).filter(|token| token.kind == TokKind::Str);
            if let Some(specifier) = specifier {
                let module = unescape_string(text(source, specifier));
                let attribute = has_file_attribute(source, tokens, cursor + 2);
                audit.module_boundary(&module, offset, attribute);
                if module == "bun:ffi" {
                    if bindings_start >= tokens.len() || text(source, &tokens[bindings_start]) != "{" {
                        audit.report(
                            offset,
                            "ffi-binding",
                            "Use named bun:ffi imports so platform loader calls remain statically visible.",
                        );
                    }
                    audit_cc_bindings(audit, source, tokens, bindings_start);
                }
            }
            break;
        }
        cursor += 1;
    }
}

/// Audit one `export ... from` re-export.
fn audit_export(audit: &mut FileAudit<'_>, source: &str, tokens: &[Token], index: usize) {
    let offset = tokens[index].start;
    let mut cursor = index + 1;
    if cursor < tokens.len() && text(source, &tokens[cursor]) == "type" {
        cursor += 1;
    }
    if cursor < tokens.len() && text(source, &tokens[cursor]) == "{" {
        let mut depth: usize = 0;
        while cursor < tokens.len() {
            match text(source, &tokens[cursor]) {
                "{" => depth += 1,
                "}" => {
                    depth -= 1;
                    if depth == 0 {
                        cursor += 1;
                        break;
                    }
                }
                ";" => return,
                _ => {}
            }
            cursor += 1;
        }
    } else if cursor < tokens.len() && text(source, &tokens[cursor]) == "*" {
        cursor += 1;
    } else {
        return;
    }
    if cursor + 1 < tokens.len() && text(source, &tokens[cursor]) == "from" && tokens[cursor + 1].kind == TokKind::Str {
        let module = unescape_string(text(source, &tokens[cursor + 1]));
        audit.module_boundary(&module, offset, false);
        if module == "bun:ffi" {
            audit.report(
                offset,
                "ffi-binding",
                "Do not re-export bun:ffi; expose owned platform services instead.",
            );
        }
    }
}

/// Whether an import carries `with { type: "file" }` (donor menu-art exception).
fn has_file_attribute(source: &str, tokens: &[Token], mut cursor: usize) -> bool {
    if cursor < tokens.len() && text(source, &tokens[cursor]) == "with" {
        cursor += 1;
    } else {
        return false;
    }
    let expected = ["{", "type", ":", "\"file\"", "}"];
    for want in expected {
        if cursor >= tokens.len() {
            return false;
        }
        let word = text(source, &tokens[cursor]);
        if want == "\"file\"" {
            if tokens[cursor].kind != TokKind::Str || unescape_string(word) != "file" {
                return false;
            }
        } else if word != want {
            return false;
        }
        cursor += 1;
    }
    true
}

/// Flag `cc` bindings in a `bun:ffi` named import.
fn audit_cc_bindings(audit: &mut FileAudit<'_>, source: &str, tokens: &[Token], bindings_start: usize) {
    if bindings_start >= tokens.len() || text(source, &tokens[bindings_start]) != "{" {
        return;
    }
    let mut depth: usize = 0;
    for (index, token) in tokens.iter().enumerate().skip(bindings_start) {
        match text(source, token) {
            "{" => depth += 1,
            "}" => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            "cc" if token.kind == TokKind::Ident && depth == 1 => {
                if index == 0 || text(source, &tokens[index - 1]) != "as" {
                    audit.report(
                        token.start,
                        "native-implementation",
                        "Bun's C compiler cannot implement project behavior.",
                    );
                }
            }
            _ => {}
        }
    }
}

/// Compiler options that must be enabled (donor `required`).
const REQUIRED_OPTIONS: [&str; 12] = [
    "strict",
    "noUncheckedIndexedAccess",
    "exactOptionalPropertyTypes",
    "noImplicitOverride",
    "noImplicitReturns",
    "noFallthroughCasesInSwitch",
    "noUnusedLocals",
    "noUnusedParameters",
    "noPropertyAccessFromIndexSignature",
    "forceConsistentCasingInFileNames",
    "noEmit",
    "verbatimModuleSyntax",
];

/// Compiler options that must be disabled (donor `disabled`).
const DISABLED_OPTIONS: [&str; 6] = [
    "allowJs",
    "checkJs",
    "noCheck",
    "noResolve",
    "suppressExcessPropertyErrors",
    "suppressImplicitAnyIndexErrors",
];

/// Strict-mode options that must not be overridden (donor `strictDefaults`).
const STRICT_DEFAULTS: [&str; 9] = [
    "noImplicitAny",
    "noImplicitThis",
    "strictNullChecks",
    "strictFunctionTypes",
    "strictBindCallApply",
    "strictPropertyInitialization",
    "strictBuiltinIteratorReturn",
    "useUnknownInCatchVariables",
    "alwaysStrict",
];

/// Find `tsconfig.json` at or above `root` (donor `ts.findConfigFile`).
fn find_config(root: &Path) -> Option<PathBuf> {
    let mut directory = Some(root);
    while let Some(current) = directory {
        let candidate = current.join("tsconfig.json");
        if candidate.is_file() {
            return Some(candidate);
        }
        directory = current.parent();
    }
    None
}

/// Merge `compilerOptions` over an `extends` chain (donor `parseJsonConfigFileContent` subset).
fn read_compiler_options(config_path: &Path) -> Result<Json, ToolsError> {
    fn load(path: &Path, depth: usize, visited: &mut Vec<PathBuf>) -> Result<Json, ToolsError> {
        if depth > 10 || visited.contains(&path.to_path_buf()) {
            return Err(ToolsError::invalid(format!(
                "tsconfig extends cycle at {}",
                path.display()
            )));
        }
        visited.push(path.to_path_buf());
        let text = crate::fsutil::read_text(path)?;
        let config = parse_jsonc(&text)?;
        let mut merged = Json::object(Vec::new());
        if let Some(extends) = config.get("extends") {
            let mut bases = Vec::new();
            if let Some(name) = extends.as_str() {
                bases.push(name.to_owned());
            } else if let Some(names) = extends.as_array() {
                for name in names {
                    if let Some(name) = name.as_str() {
                        bases.push(name.to_owned());
                    }
                }
            }
            let directory = path.parent().unwrap_or(Path::new("."));
            for base in bases {
                let mut candidate = directory.join(&base);
                if candidate.extension().is_none() {
                    candidate.set_extension("json");
                }
                let parent = load(&candidate, depth + 1, visited)?;
                if let Some(options) = parent.get("compilerOptions").and_then(Json::as_object) {
                    for (key, value) in options {
                        set_option(&mut merged, key, value.clone());
                    }
                }
            }
        }
        if let Some(options) = config.get("compilerOptions").and_then(Json::as_object) {
            for (key, value) in options {
                set_option(&mut merged, key, value.clone());
            }
        }
        Ok(Json::object(vec![("compilerOptions".to_owned(), merged)]))
    }
    load(config_path, 0, &mut Vec::new()).and_then(|config| {
        config
            .get("compilerOptions")
            .cloned()
            .ok_or_else(|| ToolsError::invalid("tsconfig has no compilerOptions"))
    })
}

/// Insert or replace one merged compiler option.
fn set_option(merged: &mut Json, key: &str, value: Json) {
    if let Json::Object(pairs) = merged {
        if let Some(slot) = pairs.iter_mut().find(|(name, _)| name == key) {
            slot.1 = value;
        } else {
            pairs.push((key.to_owned(), value));
        }
    }
}

/// Audit a project root (donor `auditProject`, syntactic subset).
pub fn audit_project(root: &Path) -> Result<Vec<PolicyDiagnostic>, ToolsError> {
    let cwd = std::env::current_dir().map_err(|error| ToolsError::io("resolving current directory", error))?;
    let project_root = lexical_absolute(&cwd, &root.to_string_lossy());
    let files = discover_files(&project_root)?;
    let config_path = find_config(&project_root)
        .ok_or_else(|| ToolsError::invalid(format!("No tsconfig.json found in {}", project_root.display())))?;
    let mut source_diagnostics = Vec::new();
    let mut file_diagnostics = Vec::new();
    for file in &files {
        let meta = std::fs::symlink_metadata(file)
            .map_err(|error| ToolsError::io(format!("stating {}", file.display()), error))?;
        if meta.is_symlink() {
            file_diagnostics.push(PolicyDiagnostic {
                file: file.to_string_lossy().into_owned(),
                line: 1,
                column: 1,
                rule: "source-symlink",
                message: "Project source cannot use symlinks to bypass source discovery or leave the workspace."
                    .to_owned(),
            });
            continue;
        }
        let name = file.to_string_lossy().into_owned();
        if !name.ends_with(".ts") || has_native_header(file) {
            file_diagnostics.push(PolicyDiagnostic {
                file: name,
                line: 1,
                column: 1,
                rule: "implementation-language",
                message: "Project implementation must be .ts; native sources/binaries, JavaScript, shader and WASM artifacts are forbidden."
                    .to_owned(),
            });
            continue;
        }
        let text = crate::fsutil::read_text(file)?;
        source_diagnostics.extend(audit_source(&project_root, file, &text));
    }
    sort_diagnostics(&mut source_diagnostics);
    let mut diagnostics = source_diagnostics;
    let options = read_compiler_options(&config_path)?;
    let config = config_path.to_string_lossy().into_owned();
    for key in REQUIRED_OPTIONS {
        if options.get(key).and_then(Json::as_bool) != Some(true) {
            diagnostics.push(PolicyDiagnostic {
                file: config.clone(),
                line: 1,
                column: 1,
                rule: "compiler-policy",
                message: format!("{key} must be enabled."),
            });
        }
    }
    for key in DISABLED_OPTIONS {
        if options.get(key).and_then(Json::as_bool) == Some(true) {
            diagnostics.push(PolicyDiagnostic {
                file: config.clone(),
                line: 1,
                column: 1,
                rule: "compiler-policy",
                message: format!("{key} must be disabled."),
            });
        }
    }
    for key in STRICT_DEFAULTS {
        if options.get(key).and_then(Json::as_bool) == Some(false) {
            diagnostics.push(PolicyDiagnostic {
                file: config.clone(),
                line: 1,
                column: 1,
                rule: "compiler-policy",
                message: format!("{key} cannot override strict mode."),
            });
        }
    }
    diagnostics.extend(file_diagnostics);
    Ok(diagnostics)
}

/// Run the policy CLI over `root`, returning the process exit code.
pub fn run(args: &[String]) -> Result<i32, ToolsError> {
    let root = match args.first() {
        Some(arg) => PathBuf::from(arg),
        None => std::env::current_dir().map_err(|error| ToolsError::io("resolving current directory", error))?,
    };
    let diagnostics =
        audit_project(&root).map_err(|error| ToolsError::invalid(format!("TypeScript policy failed: {error}")))?;
    for diagnostic in &diagnostics {
        eprintln!("{}", render_diagnostic(&root, diagnostic));
    }
    if diagnostics.is_empty() {
        println!("TypeScript policy passed.");
        Ok(0)
    } else {
        Ok(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        crate::fsutil::make_temp_dir(&std::env::temp_dir(), name).expect("temp dir")
    }

    fn write_config(root: &Path, options: &str) {
        let text = format!("{{\"compilerOptions\": {{{options}}}}}");
        crate::fsutil::write_text(&root.join("tsconfig.json"), &text).expect("config");
    }

    fn full_options() -> String {
        let mut parts = Vec::new();
        for key in REQUIRED_OPTIONS {
            parts.push(format!("\"{key}\": true"));
        }
        for key in DISABLED_OPTIONS {
            parts.push(format!("\"{key}\": false"));
        }
        parts.join(", ")
    }

    fn rules(diagnostics: &[PolicyDiagnostic]) -> Vec<&'static str> {
        diagnostics.iter().map(|diagnostic| diagnostic.rule).collect()
    }

    #[test]
    fn sniffs_native_headers() {
        let directory = fixture("quake-policy-header-");
        let cases: &[(&str, &[u8], bool)] = &[
            ("elf", &[0x7f, 0x45, 0x4c, 0x46, 0, 0, 0, 0], true),
            ("mz", &[0x4d, 0x5a, 0x90, 0], true),
            ("wasm", &[0x00, 0x61, 0x73, 0x6d, 0x01, 0, 0, 0], true),
            ("mach64", &[0xcf, 0xfa, 0xed, 0xfe, 0, 0, 0, 0], true),
            ("fat", &[0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 0], true),
            ("archive", b"!<arch>\n", true),
            ("short-archive", b"!<ar", false),
            ("text", b"export const x = 1;\n", false),
            ("tiny", b"ab", false),
        ];
        for (name, bytes, expected) in cases {
            let path = directory.join(name);
            crate::fsutil::write_bytes(&path, bytes).expect("write");
            assert_eq!(has_native_header(&path), *expected, "{name}");
        }
        assert!(!has_native_header(&directory.join("missing")));
        crate::fsutil::remove_forced(&directory);
    }

    #[test]
    fn classifies_native_artifact_names() {
        for name in [
            "addon.node",
            "lib.so",
            "lib.so.1",
            "lib.so.1.2",
            "x.DLL",
            "x.dylib",
            "x.a",
            "x.o",
            "x.obj",
            "x.lib",
            "x.exe",
            "x.SO.6",
        ] {
            assert!(is_native_artifact_name(name), "{name}");
        }
        for name in [
            "x.ts", "x.txt", "also", "x.sot", "x.so.txt", "x.os", "x.nodes", ".dylibx",
        ] {
            assert!(!is_native_artifact_name(name), "{name}");
        }
    }

    #[test]
    fn discovers_implementation_files() {
        let root = fixture("quake-policy-discover-");
        for directory in [
            "node_modules",
            ".git",
            "dist",
            ".artifacts",
            "src",
            "src/nested",
            "tools",
        ] {
            std::fs::create_dir_all(root.join(directory)).expect("mkdir");
        }
        crate::fsutil::write_text(&root.join("node_modules/skip.ts"), "x").expect("write");
        crate::fsutil::write_text(&root.join("dist/skip.ts"), "x").expect("write");
        crate::fsutil::write_text(&root.join("src/keep.ts"), "export const x = 1;\n").expect("write");
        crate::fsutil::write_text(&root.join("src/nested/deep.ts"), "x").expect("write");
        crate::fsutil::write_text(&root.join("tools/run.js"), "x").expect("write");
        crate::fsutil::write_text(&root.join("src/readme.md"), "x").expect("write");
        crate::fsutil::write_text(&root.join("src/image.png"), "x").expect("write");
        crate::fsutil::write_text(&root.join("src/addon.node"), "x").expect("write");
        crate::fsutil::write_bytes(&root.join("src/blob"), &[0x7f, 0x45, 0x4c, 0x46]).expect("write");
        let files = discover_files(&root).expect("discover");
        let names: Vec<String> = files.iter().map(|path| posix_relative(&root, path)).collect();
        assert_eq!(
            names,
            vec![
                "src/addon.node",
                "src/blob",
                "src/keep.ts",
                "src/nested/deep.ts",
                "tools/run.js",
            ]
        );
        crate::fsutil::remove_forced(&root);
    }

    #[test]
    fn flags_suppressions_at_word_boundaries() {
        let root = PathBuf::from("/repo");
        let file = root.join("src/a.ts");
        let diagnostics = audit_source(
            &root,
            &file,
            "const x = 1; // @ts-ignore\n/* @ts-expect-error */\n// @ts-nocheck: off\n// eslint-disable-next-line no-any\n// eslint-disable-line\n",
        );
        assert_eq!(rules(&diagnostics), vec!["suppression"; 5]);
        assert_eq!(diagnostics[0].line, 1);
        let clean = audit_source(
            &root,
            &file,
            "// @ts-ignoreX\n// eslint-disablefoo\n// @ts-ignoree\nconst x = 1;\n",
        );
        assert!(clean.is_empty(), "{clean:?}");
    }

    #[test]
    fn flags_assertion_keywords() {
        let root = PathBuf::from("/repo");
        let file = root.join("src/a.ts");
        let diagnostics = audit_source(
            &root,
            &file,
            "const x: any = value as string;\nconst y = input!;\nlet owned!: string;\nfunction guard(value: unknown): asserts value {}\nlet any = 1;\n",
        );
        assert_eq!(
            rules(&diagnostics),
            vec![
                "explicit-any",
                "assertion",
                "non-null",
                "definite-assignment",
                "assertion"
            ]
        );
        let clean = audit_source(
            &root,
            &file,
            "import { a as b } from \"./b.ts\";\nexport { c as d };\nconst ok = a != b;\nconst sure = !!flag;\nconst member = options.any;\nconst keyed = { any: 1 };\ndeclare nothing;\nreturn !flag && !other;\nconst negated = !value;\nif (!ready) { boot(); }\n",
        );
        assert_eq!(rules(&clean), vec!["ambient"]);
    }

    #[test]
    fn distinguishes_value_any_from_type_any() {
        let root = PathBuf::from("/repo");
        let file = root.join("src/a.ts");
        let clean = audit_source(
            &root,
            &file,
            "if (!ready && any && other) { run(anything); }\nconst total = count + any;\nconst picked = any ?? fallback;\n",
        );
        assert!(clean.is_empty(), "{clean:?}");
        let flagged = audit_source(
            &root,
            &file,
            "type Alias = any;\nconst list: Array<any> = [];\nfunction take(input: any): void {}\n",
        );
        assert_eq!(rules(&flagged), vec!["explicit-any", "explicit-any", "explicit-any"]);
    }

    #[test]
    fn flags_declare_and_declaration_files() {
        let root = PathBuf::from("/repo");
        let ambient = audit_source(&root, &root.join("src/a.ts"), "declare const x: number;\n");
        assert_eq!(rules(&ambient), vec!["ambient"]);
        let stubs = audit_source(&root, &root.join("src/stubs.d.ts"), "export declare const x: number;\n");
        assert_eq!(rules(&stubs), vec!["ambient"]);
        let multi = audit_source(
            &root,
            &root.join("src/stubs.d.ts"),
            "export declare const a: number;\nexport declare const b: number;\n",
        );
        assert_eq!(rules(&multi), vec!["ambient", "ambient"]);
        let methods = audit_source(
            &root,
            &root.join("src/a.ts"),
            "class C {\n  async declare(epoch: number): Promise<void> {}\n  private declare(path: string): string { return path; }\n}\n",
        );
        assert!(methods.is_empty(), "{methods:?}");
        let png = audit_source(
            &root,
            &root.join("src/types/png.d.ts"),
            "declare module \"*.png\" {\n  const path: string;\n  export default path;\n}\n",
        );
        assert!(png.is_empty(), "{png:?}");
    }

    #[test]
    fn checks_module_boundaries() {
        let root = PathBuf::from("/repo");
        let file = root.join("src/a.ts");
        let diagnostics = audit_source(
            &root,
            &file,
            "import \"./addon.node\";\nimport { dlopen } from \"bun:ffi\";\nimport cp from \"child_process\";\nimport outside from \"../tools/x.ts\";\nimport built from \"./dist-out\";\n",
        );
        assert_eq!(
            rules(&diagnostics),
            vec![
                "native-implementation",
                "ffi-boundary",
                "runtime-subprocess",
                "runtime-boundary"
            ]
        );
        let clean = audit_source(
            &root,
            &root.join("src/platform/sdl.ts"),
            "import { dlopen } from \"bun:ffi\";\nimport sibling from \"./other.ts\";\nimport fs from \"node:fs\";\n",
        );
        assert!(clean.is_empty(), "{clean:?}");
        let escaping = audit_source(&root, &file, "import x from \"../../escape.ts\";\n");
        assert_eq!(rules(&escaping), vec!["source-boundary", "runtime-boundary"]);
        let dynamic = audit_source(
            &root,
            &file,
            "const name = \"./x.ts\";\nawait import(name);\nawait import(\"./fine.ts\");\n",
        );
        assert_eq!(rules(&dynamic), vec!["runtime-boundary"]);
        let ffi_dynamic = audit_source(&root, &file, "await import(\"bun:ffi\");\n");
        assert_eq!(rules(&ffi_dynamic), vec!["ffi-boundary", "ffi-binding"]);
        let reexport = audit_source(&root, &file, "export * from \"bun:ffi\";\n");
        assert_eq!(rules(&reexport), vec!["ffi-boundary", "ffi-binding"]);
        let cc = audit_source(
            &root,
            &root.join("src/platform/x.ts"),
            "import { cc, dlopen } from \"bun:ffi\";\n",
        );
        assert_eq!(rules(&cc), vec!["native-implementation"]);
        let default_ffi = audit_source(&root, &root.join("src/platform/x.ts"), "import ffi from \"bun:ffi\";\n");
        assert_eq!(rules(&default_ffi), vec!["ffi-binding"]);
    }

    #[test]
    fn allows_menu_art_file_imports() {
        let root = PathBuf::from("/repo");
        let file = root.join("src/app/bootstrap/menu-art.ts");
        let allowed = audit_source(
            &root,
            &file,
            "import background from \"../../../assets/ui/menu-background.png\" with { type: \"file\" };\n",
        );
        assert!(allowed.is_empty(), "{allowed:?}");
        let denied = audit_source(
            &root,
            &file,
            "import background from \"../../../assets/ui/menu-background.png\";\n",
        );
        assert_eq!(rules(&denied), vec!["runtime-boundary"]);
    }

    #[test]
    fn checks_compiler_options() {
        let root = fixture("quake-policy-config-");
        crate::fsutil::write_text(&root.join("src-a.ts"), "export const x = 1;\n").expect("write");
        write_config(&root, "\"strict\": true");
        let diagnostics = audit_project(&root).expect("audit");
        let messages: Vec<String> = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.clone())
            .collect();
        assert!(messages.contains(&"noEmit must be enabled.".to_owned()), "{messages:?}");
        assert!(
            !messages.iter().any(|message| message.starts_with("strict ")),
            "{messages:?}"
        );
        write_config(
            &root,
            &format!("{}, \"allowJs\": true, \"strictNullChecks\": false", full_options()),
        );
        let diagnostics = audit_project(&root).expect("audit");
        let messages: Vec<String> = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.clone())
            .collect();
        assert!(
            messages.contains(&"allowJs must be disabled.".to_owned()),
            "{messages:?}"
        );
        assert!(
            messages.contains(&"strictNullChecks cannot override strict mode.".to_owned()),
            "{messages:?}"
        );
        crate::fsutil::remove_forced(&root);
    }

    #[test]
    fn merges_tsconfig_extends() {
        let root = fixture("quake-policy-extends-");
        crate::fsutil::write_text(&root.join("base.json"), "{\"compilerOptions\": {\"strict\": true}}").expect("write");
        crate::fsutil::write_text(
            &root.join("tsconfig.json"),
            "{\"extends\": \"./base.json\", \"compilerOptions\": {\"noEmit\": true}}",
        )
        .expect("write");
        let options = read_compiler_options(&root.join("tsconfig.json")).expect("options");
        assert_eq!(options.get("strict").and_then(Json::as_bool), Some(true));
        assert_eq!(options.get("noEmit").and_then(Json::as_bool), Some(true));
        crate::fsutil::write_text(&root.join("tsconfig.json"), "{\"extends\": \"./tsconfig.json\"}").expect("write");
        assert!(read_compiler_options(&root.join("tsconfig.json")).is_err());
        crate::fsutil::remove_forced(&root);
    }

    #[test]
    fn rejects_symlinks_and_foreign_implementation() {
        let root = fixture("quake-policy-files-");
        std::fs::create_dir_all(root.join("src")).expect("mkdir");
        crate::fsutil::write_text(&root.join("src/real.ts"), "export const x = 1;\n").expect("write");
        crate::fsutil::create_symlink(&root.join("src/real.ts"), &root.join("src/alias.ts")).expect("link");
        crate::fsutil::write_text(&root.join("src/run.js"), "x").expect("write");
        write_config(&root, &full_options());
        let diagnostics = audit_project(&root).expect("audit");
        assert_eq!(rules(&diagnostics), vec!["source-symlink", "implementation-language"]);
        assert_eq!(
            render_diagnostic(&root, &diagnostics[1]),
            "src/run.js:1:1 [implementation-language] Project implementation must be .ts; native sources/binaries, JavaScript, shader and WASM artifacts are forbidden."
        );
        crate::fsutil::remove_forced(&root);
    }

    #[test]
    fn missing_config_fails() {
        let root = fixture("quake-policy-noconfig-");
        let error = audit_project(&root).expect_err("config");
        assert!(error.to_string().contains("No tsconfig.json"), "{error}");
        crate::fsutil::remove_forced(&root);
    }

    #[test]
    fn clean_project_passes() {
        let root = fixture("quake-policy-clean-");
        std::fs::create_dir_all(root.join("src")).expect("mkdir");
        crate::fsutil::write_text(
            &root.join("src/a.ts"),
            "import { b } from \"./b.ts\";\nexport const a = b;\n",
        )
        .expect("write");
        crate::fsutil::write_text(&root.join("src/b.ts"), "export const b = 1;\n").expect("write");
        write_config(&root, &full_options());
        assert!(audit_project(&root).expect("audit").is_empty());
        crate::fsutil::remove_forced(&root);
    }
}
