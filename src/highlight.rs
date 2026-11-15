#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Plain,
    Keyword,
    Builtin,
    String,
    Comment,
    Number,
    Decorator,
}

impl TokenKind {
    pub fn color(self) -> &'static str {
        match self {
            TokenKind::Plain => "#e2e8f0",
            TokenKind::Keyword => "#c084fc",
            TokenKind::Builtin => "#38bdf8",
            TokenKind::String => "#86efac",
            TokenKind::Comment => "#64748b",
            TokenKind::Number => "#fca5a5",
            TokenKind::Decorator => "#fbbf24",
        }
    }

    pub fn css_class(self) -> &'static str {
        match self {
            TokenKind::Plain => "",
            TokenKind::Keyword => "tok-keyword",
            TokenKind::Builtin => "tok-builtin",
            TokenKind::String => "tok-string",
            TokenKind::Comment => "tok-comment",
            TokenKind::Number => "tok-number",
            TokenKind::Decorator => "tok-decorator",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeToken {
    pub text: String,
    pub kind: TokenKind,
}

enum LangKind {
    Python,
    TypeScript,
    Rust,
    Go,
}

impl LangKind {
    fn from_path(file_path: Option<&str>) -> Self {
        let ext = file_path
            .and_then(|p| p.rsplit('.').next())
            .map(|s| s.to_ascii_lowercase());
        match ext.as_deref() {
            Some("ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs") => LangKind::TypeScript,
            Some("rs") => LangKind::Rust,
            Some("go") => LangKind::Go,
            _ => LangKind::Python,
        }
    }

    fn hash_comments(&self) -> bool {
        matches!(self, LangKind::Python)
    }

    fn classify_ident(&self, ident: &str) -> TokenKind {
        match self {
            LangKind::Python => {
                if matches!(
                    ident,
                    "False" | "None" | "True" | "and" | "as" | "assert" | "async" | "await"
                        | "break" | "class" | "continue" | "def" | "del" | "elif" | "else"
                        | "except" | "finally" | "for" | "from" | "global" | "if" | "import"
                        | "in" | "is" | "lambda" | "nonlocal" | "not" | "or" | "pass" | "raise"
                        | "return" | "try" | "while" | "with" | "yield"
                ) {
                    TokenKind::Keyword
                } else if matches!(
                    ident,
                    "abs" | "all" | "any" | "bool" | "dict" | "enumerate" | "filter" | "float"
                        | "int" | "isinstance" | "len" | "list" | "map" | "max" | "min"
                        | "print" | "range" | "repr" | "round" | "set" | "sorted" | "str"
                        | "sum" | "tuple" | "type" | "zip" | "self" | "cls"
                ) {
                    TokenKind::Builtin
                } else {
                    TokenKind::Plain
                }
            }
            LangKind::TypeScript => {
                if matches!(
                    ident,
                    "as" | "async" | "await" | "break" | "case" | "catch" | "class" | "const"
                        | "continue" | "debugger" | "default" | "delete" | "do" | "else"
                        | "enum" | "export" | "extends" | "false" | "finally" | "for" | "from"
                        | "function" | "if" | "implements" | "import" | "in" | "instanceof"
                        | "interface" | "let" | "new" | "null" | "of" | "package" | "private"
                        | "protected" | "public" | "readonly" | "return" | "static" | "super"
                        | "switch" | "this" | "throw" | "true" | "try" | "type" | "typeof"
                        | "undefined" | "var" | "void" | "while" | "with" | "yield"
                ) {
                    TokenKind::Keyword
                } else if matches!(
                    ident,
                    "Array" | "Boolean" | "Error" | "Map" | "Math" | "Number" | "Object"
                        | "Promise" | "Record" | "Set" | "String" | "console" | "document"
                        | "window" | "never" | "unknown" | "string" | "number" | "boolean"
                        | "any"
                ) {
                    TokenKind::Builtin
                } else {
                    TokenKind::Plain
                }
            }
            LangKind::Rust => {
                if matches!(
                    ident,
                    "as" | "async" | "await" | "break" | "const" | "continue" | "crate" | "dyn"
                        | "else" | "enum" | "extern" | "false" | "fn" | "for" | "if" | "impl"
                        | "in" | "let" | "loop" | "match" | "mod" | "move" | "mut" | "pub"
                        | "ref" | "return" | "self" | "Self" | "static" | "struct" | "super"
                        | "trait" | "true" | "type" | "unsafe" | "use" | "where" | "while"
                ) {
                    TokenKind::Keyword
                } else if matches!(
                    ident,
                    "bool" | "char" | "f32" | "f64" | "i8" | "i16" | "i32" | "i64" | "isize"
                        | "str" | "u8" | "u16" | "u32" | "u64" | "usize" | "Option" | "Result"
                        | "Some" | "None" | "Ok" | "Err" | "Vec" | "String" | "Box" | "Arc"
                        | "Rc" | "println" | "format"
                ) {
                    TokenKind::Builtin
                } else {
                    TokenKind::Plain
                }
            }
            LangKind::Go => {
                if matches!(
                    ident,
                    "break" | "case" | "chan" | "const" | "continue" | "default" | "defer"
                        | "else" | "fallthrough" | "for" | "func" | "go" | "goto" | "if"
                        | "import" | "interface" | "map" | "package" | "range" | "return"
                        | "select" | "struct" | "switch" | "type" | "var" | "true" | "false"
                        | "nil" | "iota"
                ) {
                    TokenKind::Keyword
                } else if matches!(
                    ident,
                    "append" | "cap" | "close" | "complex" | "copy" | "delete" | "imag" | "len"
                        | "make" | "new" | "panic" | "print" | "println" | "real" | "recover"
                        | "error" | "string" | "int" | "int64" | "float64" | "bool" | "byte"
                        | "rune" | "any"
                ) {
                    TokenKind::Builtin
                } else {
                    TokenKind::Plain
                }
            }
        }
    }
}

fn push_token(tokens: &mut Vec<CodeToken>, text: &str, kind: TokenKind) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = tokens.last_mut() {
        if last.kind == kind {
            last.text.push_str(text);
            return;
        }
    }
    tokens.push(CodeToken {
        text: text.to_string(),
        kind,
    });
}

pub fn tokenize_code_line(line: &str, file_path: Option<&str>) -> Vec<CodeToken> {
    let lang = LangKind::from_path(file_path);
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut tokens = Vec::new();

    while i < len {
        let b = bytes[i];

        // Comments & Rust attributes #[...]
        if b == b'#' {
            if bytes.get(i + 1) == Some(&b'[') {
                if let Some(end_rel) = line[i..].find(']') {
                    let end = i + end_rel + 1;
                    push_token(&mut tokens, &line[i..end], TokenKind::Decorator);
                    i = end;
                    continue;
                }
            }
            if lang.hash_comments() {
                push_token(&mut tokens, &line[i..], TokenKind::Comment);
                break;
            } else {
                push_token(&mut tokens, "#", TokenKind::Plain);
                i += 1;
                continue;
            }
        }
        if b == b'/' && bytes.get(i + 1) == Some(&b'/') {
            push_token(&mut tokens, &line[i..], TokenKind::Comment);
            break;
        }

        // Triple-quoted or single/double/backtick strings
        if b == b'"' || b == b'\'' || b == b'`' {
            let quote = b;
            let triple = (quote == b'"' || quote == b'\'')
                && bytes.get(i + 1) == Some(&quote)
                && bytes.get(i + 2) == Some(&quote);
            let mut end = if triple { i + 3 } else { i + 1 };
            while end < len {
                if triple {
                    if bytes[end] == quote
                        && bytes.get(end + 1) == Some(&quote)
                        && bytes.get(end + 2) == Some(&quote)
                    {
                        end += 3;
                        break;
                    }
                    end += 1;
                } else {
                    if bytes[end] == b'\\' && end + 1 < len {
                        end += 2;
                        continue;
                    }
                    if bytes[end] == quote {
                        end += 1;
                        break;
                    }
                    end += 1;
                }
            }
            push_token(&mut tokens, &line[i..end], TokenKind::String);
            i = end;
            continue;
        }

        // Decorators @foo.bar
        if b == b'@'
            && bytes
                .get(i + 1)
                .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
        {
            let mut end = i + 2;
            while end < len
                && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_' || bytes[end] == b'.')
            {
                end += 1;
            }
            push_token(&mut tokens, &line[i..end], TokenKind::Decorator);
            i = end;
            continue;
        }

        // Numbers
        if b.is_ascii_digit()
            && (i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_'))
        {
            let mut end = i + 1;
            while end < len && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if bytes.get(end) == Some(&b'.') && bytes.get(end + 1).is_some_and(|c| c.is_ascii_digit())
            {
                end += 2;
                while end < len && bytes[end].is_ascii_digit() {
                    end += 1;
                }
            }
            push_token(&mut tokens, &line[i..end], TokenKind::Number);
            i = end;
            continue;
        }

        // Identifiers / keywords / builtins
        if b.is_ascii_alphabetic() || b == b'_' {
            let mut end = i + 1;
            while end < len && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            let ident = &line[i..end];
            let kind = lang.classify_ident(ident);
            push_token(&mut tokens, ident, kind);
            i = end;
            continue;
        }

        // Plain character (UTF-8 safe)
        let ch_len = line[i..].chars().next().map_or(1, char::len_utf8);
        push_token(&mut tokens, &line[i..i + ch_len], TokenKind::Plain);
        i += ch_len;
    }

    tokens
}

#[cfg(test)]
mod tests {
    use super::{tokenize_code_line, TokenKind};

    #[test]
    fn highlights_python_keywords_strings_and_comments() {
        let tokens = tokenize_code_line("def foo(self, x=42): # hi", Some("a.py"));
        assert!(tokens
            .iter()
            .any(|t| t.text == "def" && t.kind == TokenKind::Keyword));
        assert!(tokens
            .iter()
            .any(|t| t.text == "self" && t.kind == TokenKind::Builtin));
        assert!(tokens
            .iter()
            .any(|t| t.text == "42" && t.kind == TokenKind::Number));
        assert!(tokens
            .iter()
            .any(|t| t.text == "# hi" && t.kind == TokenKind::Comment));
    }

    #[test]
    fn highlights_rust_attribute_and_fn() {
        let tokens = tokenize_code_line("#[tauri::command] pub fn run() // ok", Some("lib.rs"));
        assert_eq!(tokens[0].kind, TokenKind::Decorator);
        assert!(tokens
            .iter()
            .any(|t| t.text == "fn" && t.kind == TokenKind::Keyword));
        assert!(tokens
            .iter()
            .any(|t| t.text == "// ok" && t.kind == TokenKind::Comment));
    }
}
