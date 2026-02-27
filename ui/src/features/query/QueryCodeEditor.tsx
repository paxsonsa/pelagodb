import { useMemo } from "react";
import CodeMirror from "@uiw/react-codemirror";
import { javascript } from "@codemirror/lang-javascript";
import { sql } from "@codemirror/lang-sql";
import { autocompletion, type CompletionContext } from "@codemirror/autocomplete";
import { EditorView } from "@codemirror/view";

type EditorMode = "cel" | "pql";

const CEL_KEYWORDS = [
  "true",
  "false",
  "null",
  "in",
  "contains",
  "startsWith",
  "endsWith",
  "matches",
  "size",
  "has",
  "all",
  "exists",
  "exists_one",
  "filter",
  "map",
];

const PQL_KEYWORDS = [
  "filter",
  "sort",
  "limit",
  "offset",
  "uid",
  "name",
  "age",
  "status",
  "priority",
  "stage",
  "where",
  "and",
  "or",
  "not",
];

function completionFor(mode: EditorMode, context: CompletionContext) {
  const word = context.matchBefore(/[@A-Za-z_][\w-]*/);
  if (!word) {
    return null;
  }

  if (word.from === word.to && !context.explicit) {
    return null;
  }

  const options = (mode === "cel" ? CEL_KEYWORDS : PQL_KEYWORDS).map((label) => ({
    label,
    type: "keyword",
  }));

  return {
    from: word.from,
    options,
  };
}

export function QueryCodeEditor({
  mode,
  value,
  onChange,
  minHeight,
  placeholder,
}: {
  mode: EditorMode;
  value: string;
  onChange: (value: string) => void;
  minHeight: number;
  placeholder?: string;
}) {
  const extensions = useMemo(() => {
    const language = mode === "pql" ? sql() : javascript({ typescript: false });
    return [
      language,
      autocompletion({ override: [(context) => completionFor(mode, context)] }),
      EditorView.lineWrapping,
      EditorView.theme({
        "&": {
          borderRadius: "0.375rem",
          border: "1px solid var(--border)",
          backgroundColor: "var(--panel)",
          fontSize: "13px",
        },
        ".cm-scroller": {
          minHeight: `${minHeight}px`,
          fontFamily: "var(--font-mono)",
        },
        ".cm-content": {
          padding: "10px",
        },
        ".cm-focused": {
          outline: "none",
        },
        ".cm-editor.cm-focused": {
          boxShadow: "0 0 0 2px var(--ring)",
        },
      }),
    ];
  }, [minHeight, mode]);

  return (
    <CodeMirror
      value={value}
      basicSetup={{
        autocompletion: true,
        bracketMatching: true,
        foldGutter: false,
      }}
      extensions={extensions}
      onChange={onChange}
      placeholder={placeholder}
    />
  );
}
