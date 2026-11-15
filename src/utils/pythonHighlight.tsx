import React from "react";

export type TokenKind =
  | "plain"
  | "keyword"
  | "builtin"
  | "string"
  | "comment"
  | "number"
  | "decorator";

export interface CodeToken {
  text: string;
  kind: TokenKind;
}

export const TOKEN_COLORS: Record<TokenKind, string> = {
  plain: "#e2e8f0",
  keyword: "#c084fc",
  builtin: "#38bdf8",
  string: "#86efac",
  comment: "#64748b",
  number: "#fca5a5",
  decorator: "#fbbf24",
};

const PY_KEYWORDS = new Set([
  "False", "None", "True", "and", "as", "assert", "async", "await", "break",
  "class", "continue", "def", "del", "elif", "else", "except", "finally",
  "for", "from", "global", "if", "import", "in", "is", "lambda", "nonlocal",
  "not", "or", "pass", "raise", "return", "try", "while", "with", "yield",
]);

const PY_BUILTINS = new Set([
  "abs", "all", "any", "bool", "dict", "enumerate", "filter", "float", "int",
  "isinstance", "len", "list", "map", "max", "min", "print", "range", "repr",
  "round", "set", "sorted", "str", "sum", "tuple", "type", "zip", "self", "cls",
]);

const TS_KEYWORDS = new Set([
  "as", "async", "await", "break", "case", "catch", "class", "const", "continue",
  "debugger", "default", "delete", "do", "else", "enum", "export", "extends",
  "false", "finally", "for", "from", "function", "if", "implements", "import",
  "in", "instanceof", "interface", "let", "new", "null", "of", "package",
  "private", "protected", "public", "readonly", "return", "static", "super",
  "switch", "this", "throw", "true", "try", "type", "typeof", "undefined",
  "var", "void", "while", "with", "yield",
]);

const TS_BUILTINS = new Set([
  "Array", "Boolean", "Error", "Map", "Math", "Number", "Object", "Promise",
  "Record", "Set", "String", "console", "document", "window", "never", "unknown",
  "string", "number", "boolean", "any",
]);

const RS_KEYWORDS = new Set([
  "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else",
  "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop",
  "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static",
  "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while",
]);

const RS_BUILTINS = new Set([
  "bool", "char", "f32", "f64", "i8", "i16", "i32", "i64", "isize", "str",
  "u8", "u16", "u32", "u64", "usize", "Option", "Result", "Some", "None", "Ok",
  "Err", "Vec", "String", "Box", "Arc", "Rc", "println", "format",
]);

const GO_KEYWORDS = new Set([
  "break", "case", "chan", "const", "continue", "default", "defer", "else",
  "fallthrough", "for", "func", "go", "goto", "if", "import", "interface",
  "map", "package", "range", "return", "select", "struct", "switch", "type",
  "var", "true", "false", "nil", "iota",
]);

const GO_BUILTINS = new Set([
  "append", "cap", "close", "complex", "copy", "delete", "imag", "len", "make",
  "new", "panic", "print", "println", "real", "recover", "error", "string",
  "int", "int64", "float64", "bool", "byte", "rune", "any",
]);

const TOKEN_RE =
  /(#[^\n]*|\/\/[^\n]*)|("""[\s\S]*?"""|'''[\s\S]*?'''|"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|`[^`]*`)|(@[A-Za-z_][A-Za-z0-9_.]*|#\[[^\]]*\])|(\b\d+(?:\.\d+)?\b)|(\b[A-Za-z_][A-Za-z0-9_]*\b)/g;

function langSets(filePath?: string): { keywords: Set<string>; builtins: Set<string>; hashComments: boolean } {
  const ext = filePath?.split(".").pop()?.toLowerCase() ?? "py";
  if (ext === "ts" || ext === "tsx" || ext === "js" || ext === "jsx" || ext === "mjs" || ext === "cjs") {
    return { keywords: TS_KEYWORDS, builtins: TS_BUILTINS, hashComments: false };
  }
  if (ext === "rs") {
    return { keywords: RS_KEYWORDS, builtins: RS_BUILTINS, hashComments: false };
  }
  if (ext === "go") {
    return { keywords: GO_KEYWORDS, builtins: GO_BUILTINS, hashComments: false };
  }
  return { keywords: PY_KEYWORDS, builtins: PY_BUILTINS, hashComments: true };
}

export function tokenizeCodeLine(line: string, filePath?: string): CodeToken[] {
  const { keywords, builtins, hashComments } = langSets(filePath);
  const tokens: CodeToken[] = [];
  let lastIndex = 0;

  TOKEN_RE.lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = TOKEN_RE.exec(line)) !== null) {
    if (match.index > lastIndex) {
      tokens.push({ text: line.slice(lastIndex, match.index), kind: "plain" });
    }

    const [full, comment, str, decorator, num, ident] = match;
    if (comment) {
      if (full.startsWith("#") && !hashComments) {
        tokens.push({ text: full, kind: full.startsWith("#[") ? "decorator" : "plain" });
      } else {
        tokens.push({ text: full, kind: "comment" });
      }
    } else if (str) {
      tokens.push({ text: full, kind: "string" });
    } else if (decorator) {
      tokens.push({ text: full, kind: "decorator" });
    } else if (num) {
      tokens.push({ text: full, kind: "number" });
    } else if (ident) {
      if (keywords.has(ident)) {
        tokens.push({ text: full, kind: "keyword" });
      } else if (builtins.has(ident)) {
        tokens.push({ text: full, kind: "builtin" });
      } else {
        tokens.push({ text: full, kind: "plain" });
      }
    }

    lastIndex = match.index + full.length;
  }

  if (lastIndex < line.length) {
    tokens.push({ text: line.slice(lastIndex), kind: "plain" });
  }

  return tokens;
}

export function tokenizePythonLine(line: string): CodeToken[] {
  return tokenizeCodeLine(line, "file.py");
}

export function highlightPython(code: string, filePath?: string): React.ReactNode[] {
  const tokens = tokenizeCodeLine(code, filePath);
  return tokens.map((tok, idx) =>
    tok.kind === "plain" ? (
      tok.text
    ) : (
      <span key={idx} className={`tok-${tok.kind}`}>
        {tok.text}
      </span>
    )
  );
}
