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

const KEYWORDS = new Set([
  "False",
  "None",
  "True",
  "and",
  "as",
  "assert",
  "async",
  "await",
  "break",
  "class",
  "continue",
  "def",
  "del",
  "elif",
  "else",
  "except",
  "finally",
  "for",
  "from",
  "global",
  "if",
  "import",
  "in",
  "is",
  "lambda",
  "nonlocal",
  "not",
  "or",
  "pass",
  "raise",
  "return",
  "try",
  "while",
  "with",
  "yield",
]);

const BUILTINS = new Set([
  "abs",
  "all",
  "any",
  "bool",
  "dict",
  "enumerate",
  "filter",
  "float",
  "int",
  "isinstance",
  "len",
  "list",
  "map",
  "max",
  "min",
  "print",
  "range",
  "repr",
  "round",
  "set",
  "sorted",
  "str",
  "sum",
  "tuple",
  "type",
  "zip",
  "self",
  "cls",
]);

const TOKEN_RE =
  /(#[^\n]*)|("""[\s\S]*?"""|'''[\s\S]*?'''|"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*')|(@[A-Za-z_][A-Za-z0-9_.]*)|(\b\d+(?:\.\d+)?\b)|(\b[A-Za-z_][A-Za-z0-9_]*\b)/g;

export function tokenizePythonLine(line: string): CodeToken[] {
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
      tokens.push({ text: full, kind: "comment" });
    } else if (str) {
      tokens.push({ text: full, kind: "string" });
    } else if (decorator) {
      tokens.push({ text: full, kind: "decorator" });
    } else if (num) {
      tokens.push({ text: full, kind: "number" });
    } else if (ident) {
      if (KEYWORDS.has(ident)) {
        tokens.push({ text: full, kind: "keyword" });
      } else if (BUILTINS.has(ident)) {
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

export function highlightPython(code: string): React.ReactNode[] {
  const tokens = tokenizePythonLine(code);
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
