use super::DiagnosticKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Text,
    Open,
    Close,
    LeftParen,
    RightParen,
}
#[derive(Debug)]
pub(super) struct Token {
    pub text: String,
    pub kind: Kind,
    pub line: usize,
    pub newline_before: bool,
}
pub(super) struct LexError {
    pub line: usize,
    pub kind: DiagnosticKind,
}

/// COM_Compress removes comments before COM_ParseExt. In particular, line
/// breaks inside block comments do not terminate animMap/tcMod arguments.
pub(super) fn lex(bytes: &[u8]) -> Result<Vec<Token>, LexError> {
    if std::str::from_utf8(bytes).is_err() {
        return Err(LexError {
            line: 1,
            kind: DiagnosticKind::InvalidUtf8,
        });
    }
    let mut tokens = Vec::new();
    let mut at = 0;
    let mut line = 1;
    let mut newline = false;
    while at < bytes.len() {
        let byte = bytes[at];
        if byte == 0 {
            return Err(LexError {
                line,
                kind: DiagnosticKind::EmbeddedNul,
            });
        }
        if byte <= 32 {
            if byte == b'\n' {
                line += 1;
                newline = true
            } else if byte == b'\r' {
                newline = true
            }
            at += 1;
            continue;
        }
        if bytes[at..].starts_with(b"//") {
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1
            }
            continue;
        }
        if bytes[at..].starts_with(b"/*") {
            skip_block(bytes, &mut at, &mut line)?;
            continue;
        }
        let first_line = line;
        let before = newline;
        newline = false;
        let kind = match byte {
            b'{' => Kind::Open,
            b'}' => Kind::Close,
            b'(' => Kind::LeftParen,
            b')' => Kind::RightParen,
            _ => Kind::Text,
        };
        if kind != Kind::Text {
            tokens.push(Token {
                text: (byte as char).to_string(),
                kind,
                line,
                newline_before: before,
            });
            at += 1;
            continue;
        }
        let mut text = Vec::new();
        if byte == b'"' {
            at += 1;
            while at < bytes.len() && bytes[at] != b'"' {
                if bytes[at] == 0 {
                    return Err(LexError {
                        line,
                        kind: DiagnosticKind::EmbeddedNul,
                    });
                }
                if bytes[at] == b'\n' {
                    line += 1
                }
                text.push(bytes[at]);
                at += 1;
            }
            if at == bytes.len() {
                return Err(LexError {
                    line: first_line,
                    kind: DiagnosticKind::UnterminatedQuote,
                });
            }
            at += 1;
        } else {
            while at < bytes.len() {
                if bytes[at] == 0 {
                    return Err(LexError {
                        line,
                        kind: DiagnosticKind::EmbeddedNul,
                    });
                }
                if bytes[at] <= 32 || b"{}()".contains(&bytes[at]) {
                    break;
                }
                if bytes[at..].starts_with(b"//") {
                    break;
                }
                if bytes[at..].starts_with(b"/*") {
                    skip_block(bytes, &mut at, &mut line)?;
                    continue;
                }
                text.push(bytes[at]);
                at += 1;
            }
        }
        if text.len() >= 1024 {
            return Err(LexError {
                line: first_line,
                kind: DiagnosticKind::TokenTooLong,
            });
        }
        let text = String::from_utf8(text).map_err(|_| LexError {
            line: first_line,
            kind: DiagnosticKind::InvalidUtf8,
        })?;
        tokens.push(Token {
            text,
            kind,
            line: first_line,
            newline_before: before,
        });
    }
    Ok(tokens)
}
fn skip_block(bytes: &[u8], at: &mut usize, line: &mut usize) -> Result<(), LexError> {
    let first_line = *line;
    *at += 2;
    while *at < bytes.len() {
        if bytes[*at..].starts_with(b"*/") {
            *at += 2;
            return Ok(());
        }
        if bytes[*at] == b'\n' {
            *line += 1
        }
        if bytes[*at] == 0 {
            return Err(LexError {
                line: *line,
                kind: DiagnosticKind::EmbeddedNul,
            });
        }
        *at += 1;
    }
    Err(LexError {
        line: first_line,
        kind: DiagnosticKind::UnterminatedComment,
    })
}
