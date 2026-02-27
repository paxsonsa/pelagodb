use super::{ReplColorMode, ReplCompletionMode, ReplSchemaCatalog};
use rustyline::completion::{Completer, Pair};
use rustyline::highlight::{Highlighter, MatchingBracketHighlighter};
use rustyline::hint::{Hinter, HistoryHinter};
use rustyline::validate::{ValidationContext, ValidationResult, Validator};
use rustyline::{Context, Helper};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

const ANSI_RESET: &str = "\x1b[0m";
const ANSI_KEYWORD: &str = "\x1b[1;34m";
const ANSI_META: &str = "\x1b[1;36m";
const ANSI_PARAM: &str = "\x1b[1;33m";
const ANSI_DIRECTIVE: &str = "\x1b[1;35m";
const ANSI_HINT: &str = "\x1b[2m";
const ANSI_PROMPT: &str = "\x1b[1;32m";

const META_COMMANDS: &[&str] = &[
    ":help",
    ":h",
    ":quit",
    ":q",
    ":exit",
    ":use ",
    ":db ",
    ":format ",
    ":set ",
    ":param ",
    ":params",
    ":clear",
    ":history",
    ":explain ",
];

const META_SET_KEYS: &[&str] = &["completion ", "color "];
const COMPLETION_MODE_VALUES: &[&str] = &["fuzzy", "prefix"];
const COLOR_MODE_VALUES: &[&str] = &["on", "off"];
const FORMAT_VALUES: &[&str] = &["json", "table", "csv"];

const SQL_PHRASES: &[&str] = &[
    "USE ",
    "USE DATABASE ",
    "USE NAMESPACE ",
    "SHOW SCHEMAS",
    "SHOW SCHEMA ",
    "DESCRIBE ",
    "CREATE TYPE ",
    "CREATE EDGE ",
    "DROP TYPE ",
    "INSERT INTO ",
    "UPDATE ",
    "DELETE FROM ",
    "DELETE EDGE ",
    "SHOW EDGES ",
    "SELECT * FROM ",
    "TRAVERSE FROM ",
    "SHOW JOBS",
    "SHOW JOB ",
    "SHOW SITES",
    "SHOW REPLICATION STATUS",
    "SHOW NAMESPACE SETTINGS",
    "SET NAMESPACE OWNER ",
    "CLEAR NAMESPACE OWNER",
];

const SQL_KEYWORDS: &[&str] = &[
    "USE",
    "DATABASE",
    "NAMESPACE",
    "SHOW",
    "SCHEMAS",
    "SCHEMA",
    "DESCRIBE",
    "CREATE",
    "TYPE",
    "EDGE",
    "DROP",
    "INSERT",
    "INTO",
    "VALUES",
    "UPDATE",
    "SET",
    "DELETE",
    "FROM",
    "SELECT",
    "WHERE",
    "LIMIT",
    "TRAVERSE",
    "VIA",
    "DEPTH",
    "MAX_RESULTS",
    "DIRECTION",
    "LABEL",
    "OUT",
    "IN",
    "BOTH",
    "JOBS",
    "JOB",
    "SITES",
    "REPLICATION",
    "STATUS",
    "OWNER",
    "CLEAR",
];

const SQL_PREFIX_STARTERS: &[&str] = &[
    "use ",
    "show ",
    "describe ",
    "create ",
    "drop ",
    "insert ",
    "update ",
    "delete ",
    "select ",
    "traverse ",
    "set namespace ",
    "clear namespace ",
];

const PQL_ROOT_FUNCS: &[&str] = &[
    "uid(",
    "type(",
    "eq(",
    "ge(",
    "le(",
    "gt(",
    "lt(",
    "between(",
    "has(",
    "allofterms(",
];

const PQL_DIRECTIVES: &[&str] = &[
    "@filter(",
    "@edge(",
    "@cascade",
    "@limit(",
    "@sort(",
    "@facets(",
    "@recurse(",
    "@groupby(",
    "@explain",
];

const PQL_AGGREGATES: &[&str] = &["count(", "sum(", "avg(", "min(", "max("];
const PQL_EDGE_TOKENS: &[&str] = &["-[", "]->", "<-", "<->"];
const PQL_QUERY_SKELETONS: &[&str] = &["{", "result(func: type(", "result(func: uid("];
const PQL_FALLBACK_TOKENS: &[&str] = &["query ", "func:", "as ", "{", "}"];

#[derive(Default)]
pub(crate) struct ReplHelper {
    bracket_highlighter: MatchingBracketHighlighter,
    hinter: HistoryHinter,
    completion_mode: ReplCompletionMode,
    color_mode: ReplColorMode,
    entity_types: Vec<String>,
    fields_by_entity: HashMap<String, Vec<String>>,
    edges_by_entity: HashMap<String, Vec<String>>,
    all_fields: Vec<String>,
    all_edges: Vec<String>,
    params: Vec<String>,
}

impl ReplHelper {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn set_schema_catalog(&mut self, catalog: &ReplSchemaCatalog) {
        self.entity_types = dedupe_sorted(catalog.entity_types.iter().cloned());
        self.fields_by_entity = catalog.fields_by_entity.clone();
        self.edges_by_entity = catalog.edges_by_entity.clone();

        self.all_fields = dedupe_sorted(
            self.fields_by_entity
                .values()
                .flat_map(|fields| fields.iter().cloned()),
        );
        self.all_edges = dedupe_sorted(
            self.edges_by_entity
                .values()
                .flat_map(|edges| edges.iter().cloned()),
        );
    }

    pub(crate) fn set_params<'a, I>(&mut self, keys: I)
    where
        I: IntoIterator<Item = &'a String>,
    {
        let params = keys
            .into_iter()
            .map(|k| format!("${}", k))
            .collect::<Vec<_>>();
        self.params = dedupe_sorted(params);
    }

    pub(crate) fn set_completion_mode(&mut self, mode: ReplCompletionMode) {
        self.completion_mode = mode;
    }

    pub(crate) fn set_color_mode(&mut self, mode: ReplColorMode) {
        self.color_mode = mode;
    }

    fn complete_meta(&self, line: &str, pos: usize) -> (usize, Vec<Pair>) {
        let leading = line.len() - line.trim_start().len();
        let input = &line[leading..pos];

        if starts_with_ci(input, ":format ") {
            let start = leading + ":format ".len();
            return self.finalize_candidates(start, line, pos, strings(FORMAT_VALUES));
        }

        if starts_with_ci(input, ":set ") {
            return self.complete_set_meta(line, pos, leading);
        }

        if starts_with_ci(input, ":param ") {
            let start = find_word_start(line, pos);
            return self.finalize_candidates(start, line, pos, self.params.clone());
        }

        if !input.contains(char::is_whitespace) {
            return self.finalize_candidates(leading, line, pos, strings(META_COMMANDS));
        }

        let start = find_word_start(line, pos);
        let prefix = &line[start..pos];
        if prefix.starts_with('$') {
            return self.finalize_candidates(start, line, pos, self.params.clone());
        }

        (start, Vec::new())
    }

    fn complete_set_meta(&self, line: &str, pos: usize, leading: usize) -> (usize, Vec<Pair>) {
        let command_start = leading + ":set ".len();
        let rest = &line[command_start..pos];
        let trimmed = rest.trim_start();

        if trimmed.is_empty() || !trimmed.contains(char::is_whitespace) {
            return self.finalize_candidates(command_start, line, pos, strings(META_SET_KEYS));
        }

        let ws = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let key = rest[..ws].trim();
        let value_part = rest[ws..].trim_start();
        let value_start = pos.saturating_sub(value_part.len());

        if key.eq_ignore_ascii_case("completion") {
            return self.finalize_candidates(
                value_start,
                line,
                pos,
                strings(COMPLETION_MODE_VALUES),
            );
        }
        if key.eq_ignore_ascii_case("color") {
            return self.finalize_candidates(value_start, line, pos, strings(COLOR_MODE_VALUES));
        }

        self.finalize_candidates(command_start, line, pos, strings(META_SET_KEYS))
    }

    fn complete_sql(&self, line: &str, pos: usize) -> (usize, Vec<Pair>) {
        let leading = line.len() - line.trim_start().len();
        let input = &line[leading..pos];

        let phrase_matches =
            rank_candidates(input, strings(SQL_PHRASES), self.completion_mode, true);
        if !phrase_matches.is_empty() {
            return (
                leading,
                phrase_matches
                    .into_iter()
                    .map(pair_from_candidate)
                    .collect::<Vec<_>>(),
            );
        }

        let start = find_word_start(line, pos);
        let mut candidates = strings(SQL_KEYWORDS);
        candidates.extend(self.entity_types.clone());
        candidates.extend(self.params.clone());

        if starts_with_ci(input, "show ") {
            candidates.extend(strings(&[
                "SCHEMAS",
                "SCHEMA ",
                "JOBS",
                "JOB ",
                "SITES",
                "EDGES ",
                "REPLICATION STATUS",
                "NAMESPACE SETTINGS",
            ]));
        }
        if starts_with_ci(input, "use ") {
            candidates.extend(strings(&["DATABASE ", "NAMESPACE "]));
        }
        if starts_with_ci(input, "traverse ") {
            candidates.extend(strings(&["FROM ", "VIA ", "DEPTH ", "MAX_RESULTS "]));
        }
        if starts_with_ci(input, "show edges ") {
            candidates.extend(strings(&["DIRECTION ", "LABEL ", "LIMIT "]));
        }
        if starts_with_ci(input, "set namespace ") {
            candidates.extend(strings(&["OWNER "]));
        }

        self.finalize_candidates(start, line, pos, candidates)
    }

    fn complete_pql(&self, line: &str, pos: usize) -> (usize, Vec<Pair>) {
        let leading = line.len() - line.trim_start().len();
        let input = &line[leading..pos];

        if is_line_start_context(input) {
            let mut candidates = vec!["query ".to_string()];
            candidates.extend(self.entity_types.clone());
            return self.finalize_candidates(leading, line, pos, candidates);
        }

        if is_query_header_context(input) {
            let start = find_word_start(line, pos);
            return self.finalize_candidates(start, line, pos, strings(PQL_QUERY_SKELETONS));
        }

        if let Some(start) = func_token_start(input) {
            return self.finalize_candidates(leading + start, line, pos, strings(PQL_ROOT_FUNCS));
        }

        if let Some(start) = directive_token_start(input) {
            return self.finalize_candidates(leading + start, line, pos, strings(PQL_DIRECTIVES));
        }

        if let Some(start) = edge_label_start(input) {
            let entity_hint = infer_entity_hint(input);
            let mut candidates = self.edge_candidates(entity_hint.as_deref());
            if candidates.is_empty() {
                candidates = self.all_edges.clone();
            }
            return self.finalize_candidates(leading + start, line, pos, candidates);
        }

        if is_selection_context(input) {
            let start = find_word_start(line, pos);
            let entity_hint = infer_entity_hint(input);
            let mut candidates = self.field_candidates(entity_hint.as_deref());
            candidates.extend(strings(PQL_AGGREGATES));
            candidates.extend(strings(PQL_EDGE_TOKENS));
            candidates.extend(strings(PQL_DIRECTIVES));
            return self.finalize_candidates(start, line, pos, candidates);
        }

        let start = find_word_start(line, pos);
        let mut candidates = strings(PQL_FALLBACK_TOKENS);
        candidates.extend(strings(PQL_ROOT_FUNCS));
        candidates.extend(strings(PQL_DIRECTIVES));
        candidates.extend(strings(PQL_AGGREGATES));
        candidates.extend(strings(PQL_EDGE_TOKENS));
        candidates.extend(self.entity_types.clone());
        candidates.extend(self.params.clone());
        self.finalize_candidates(start, line, pos, candidates)
    }

    fn finalize_candidates(
        &self,
        start: usize,
        line: &str,
        pos: usize,
        candidates: Vec<String>,
    ) -> (usize, Vec<Pair>) {
        let start = start.min(pos).min(line.len());
        let prefix = line.get(start..pos).unwrap_or_default();
        let ranked = rank_candidates(prefix, candidates, self.completion_mode, false);
        let out = ranked.into_iter().map(pair_from_candidate).collect();
        (start, out)
    }

    fn field_candidates(&self, entity_hint: Option<&str>) -> Vec<String> {
        if let Some(entity) = entity_hint {
            if let Some(fields) = lookup_case_insensitive(&self.fields_by_entity, entity) {
                return fields.clone();
            }
        }
        self.all_fields.clone()
    }

    fn edge_candidates(&self, entity_hint: Option<&str>) -> Vec<String> {
        if let Some(entity) = entity_hint {
            if let Some(edges) = lookup_case_insensitive(&self.edges_by_entity, entity) {
                return edges.clone();
            }
        }
        self.all_edges.clone()
    }
}

impl Completer for ReplHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let pos = pos.min(line.len());
        let input = &line[..pos];
        if input.trim_start().starts_with(':') {
            let (start, candidates) = self.complete_meta(input, pos);
            return Ok((start, candidates));
        }

        if is_likely_pql(input) {
            let (start, candidates) = self.complete_pql(input, pos);
            return Ok((start, candidates));
        }

        let (start, candidates) = self.complete_sql(input, pos);
        Ok((start, candidates))
    }
}

impl Hinter for ReplHelper {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, ctx: &Context<'_>) -> Option<Self::Hint> {
        self.hinter.hint(line, pos, ctx)
    }
}

impl Highlighter for ReplHelper {
    fn highlight<'l>(&self, line: &'l str, pos: usize) -> Cow<'l, str> {
        if self.color_mode == ReplColorMode::Off {
            return Cow::Borrowed(line);
        }
        match self.bracket_highlighter.highlight(line, pos) {
            Cow::Owned(bracketed) => Cow::Owned(bracketed),
            Cow::Borrowed(_) => {
                let highlighted = highlight_line(line);
                if highlighted == line {
                    Cow::Borrowed(line)
                } else {
                    Cow::Owned(highlighted)
                }
            }
        }
    }

    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        _default: bool,
    ) -> Cow<'b, str> {
        if self.color_mode == ReplColorMode::Off {
            Cow::Borrowed(prompt)
        } else {
            Cow::Owned(format!("{ANSI_PROMPT}{prompt}{ANSI_RESET}"))
        }
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        if self.color_mode == ReplColorMode::Off {
            Cow::Borrowed(hint)
        } else {
            Cow::Owned(format!("{ANSI_HINT}{hint}{ANSI_RESET}"))
        }
    }

    fn highlight_candidate<'c>(
        &self,
        candidate: &'c str,
        _completion: rustyline::CompletionType,
    ) -> Cow<'c, str> {
        if self.color_mode == ReplColorMode::Off {
            Cow::Borrowed(candidate)
        } else {
            Cow::Owned(format!("{ANSI_META}{candidate}{ANSI_RESET}"))
        }
    }

    fn highlight_char(&self, line: &str, pos: usize, forced: bool) -> bool {
        if self.color_mode == ReplColorMode::Off {
            return false;
        }
        self.bracket_highlighter.highlight_char(line, pos, forced) || !line.is_empty()
    }
}

impl Validator for ReplHelper {
    fn validate(&self, ctx: &mut ValidationContext) -> rustyline::Result<ValidationResult> {
        if needs_continuation(ctx.input()) {
            Ok(ValidationResult::Incomplete)
        } else {
            Ok(ValidationResult::Valid(None))
        }
    }
}

impl Helper for ReplHelper {}

pub(crate) fn needs_continuation(input: &str) -> bool {
    let mut brace_depth = 0i32;
    let mut paren_depth = 0i32;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escaped = false;

    for ch in input.chars() {
        if escaped {
            escaped = false;
            continue;
        }

        if ch == '\\' {
            escaped = true;
            continue;
        }

        if ch == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            continue;
        }
        if ch == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            continue;
        }

        if in_single_quote || in_double_quote {
            continue;
        }

        match ch {
            '{' => brace_depth += 1,
            '}' => brace_depth -= 1,
            '(' => paren_depth += 1,
            ')' => paren_depth -= 1,
            _ => {}
        }
    }

    if brace_depth > 0 || paren_depth > 0 || in_single_quote || in_double_quote {
        return true;
    }

    let statement = input.trim_end();
    if (starts_with_ci(statement, "create type ") || starts_with_ci(statement, "create schema "))
        && !statement.ends_with(';')
    {
        return true;
    }

    false
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).to_string()).collect()
}

fn pair_from_candidate(candidate: String) -> Pair {
    Pair {
        display: candidate.trim_end().to_string(),
        replacement: candidate,
    }
}

fn rank_candidates(
    prefix: &str,
    candidates: Vec<String>,
    mode: ReplCompletionMode,
    strict_prefix: bool,
) -> Vec<String> {
    let prefix = prefix.trim_start();
    let query = prefix.to_ascii_lowercase();
    let mut seen = HashSet::new();
    let mut scored = Vec::new();

    for candidate in candidates {
        if candidate.trim().is_empty() {
            continue;
        }
        let key = candidate.to_ascii_lowercase();
        if !seen.insert(key.clone()) {
            continue;
        }

        let Some(score) = completion_score(&key, &query, mode, strict_prefix) else {
            continue;
        };
        scored.push((score, candidate));
    }

    scored.sort_by(|(a_score, a), (b_score, b)| {
        a_score
            .cmp(b_score)
            .then_with(|| a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase()))
    });

    scored.into_iter().map(|(_, candidate)| candidate).collect()
}

fn completion_score(
    candidate_lower: &str,
    query_lower: &str,
    mode: ReplCompletionMode,
    strict_prefix: bool,
) -> Option<(u8, usize, usize, usize)> {
    if query_lower.is_empty() {
        return Some((0, 0, 0, candidate_lower.len()));
    }

    if candidate_lower.starts_with(query_lower) {
        return Some((0, 0, 0, candidate_lower.len()));
    }

    if strict_prefix || mode == ReplCompletionMode::Prefix {
        return None;
    }

    if let Some(idx) = candidate_lower.find(query_lower) {
        return Some((1, idx, query_lower.len(), candidate_lower.len()));
    }

    if let Some((gap_penalty, span)) = subsequence_metrics(candidate_lower, query_lower) {
        return Some((2, gap_penalty, span, candidate_lower.len()));
    }

    None
}

fn subsequence_metrics(candidate: &str, query: &str) -> Option<(usize, usize)> {
    if query.is_empty() {
        return Some((0, 0));
    }

    let query_chars: Vec<char> = query.chars().collect();
    let mut query_idx = 0usize;
    let mut first_match = None;
    let mut prev_match = None;
    let mut last_match = None;
    let mut gap_penalty = 0usize;

    for (idx, ch) in candidate.chars().enumerate() {
        if query_idx >= query_chars.len() {
            break;
        }
        if ch == query_chars[query_idx] {
            if first_match.is_none() {
                first_match = Some(idx);
            }
            if let Some(prev) = prev_match {
                gap_penalty += idx.saturating_sub(prev + 1);
            }
            prev_match = Some(idx);
            last_match = Some(idx);
            query_idx += 1;
        }
    }

    if query_idx != query_chars.len() {
        return None;
    }

    let start = first_match?;
    let end = last_match?;
    Some((gap_penalty, end.saturating_sub(start) + 1))
}

fn is_likely_pql(input: &str) -> bool {
    let trimmed = input.trim_start();
    if trimmed.is_empty() {
        return true;
    }
    if looks_like_sql(trimmed) {
        return false;
    }
    starts_with_ci(trimmed, "query")
        || trimmed.contains("func:")
        || trimmed.contains('@')
        || trimmed.contains("-[")
        || trimmed.contains("uid(")
        || trimmed.contains("type(")
        || trimmed.contains('{')
}

fn looks_like_sql(input: &str) -> bool {
    SQL_PREFIX_STARTERS
        .iter()
        .any(|starter| starts_with_ci(input, starter))
}

fn is_line_start_context(input: &str) -> bool {
    let trimmed = input.trim_start();
    if trimmed.is_empty() {
        return true;
    }
    !trimmed.contains(char::is_whitespace)
        && !trimmed.contains('{')
        && !trimmed.contains('(')
        && !trimmed.contains('@')
        && !trimmed.contains("-[")
}

fn is_query_header_context(input: &str) -> bool {
    let trimmed = input.trim_start();
    starts_with_ci(trimmed, "query") && !trimmed.contains('{')
}

fn func_token_start(input: &str) -> Option<usize> {
    let lower = input.to_ascii_lowercase();
    let idx = lower.rfind("func:")?;
    let mut start = idx + "func:".len();
    while let Some(ch) = input[start..].chars().next() {
        if !ch.is_whitespace() {
            break;
        }
        start += ch.len_utf8();
    }
    if input[start..].contains(')') {
        return None;
    }
    Some(start)
}

fn directive_token_start(input: &str) -> Option<usize> {
    let idx = input.rfind('@')?;
    let tail = &input[idx..];
    if tail.contains(char::is_whitespace) || tail.contains('{') || tail.contains('}') {
        return None;
    }
    Some(idx)
}

fn edge_label_start(input: &str) -> Option<usize> {
    let idx = input.rfind("-[")?;
    let tail = &input[idx + 2..];
    if tail.contains(']') {
        return None;
    }
    Some(idx + 2)
}

fn is_selection_context(input: &str) -> bool {
    if func_token_start(input).is_some() || directive_token_start(input).is_some() {
        return false;
    }
    if edge_label_start(input).is_some() {
        return false;
    }
    brace_depth(input) > 0
}

fn brace_depth(input: &str) -> i32 {
    let mut depth = 0i32;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escaped = false;
    for ch in input.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            continue;
        }
        if ch == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            continue;
        }
        if in_single_quote || in_double_quote {
            continue;
        }
        match ch {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
    }
    depth
}

fn infer_entity_hint(input: &str) -> Option<String> {
    let trimmed = input.trim_start();
    if trimmed.is_empty() {
        return None;
    }

    if !starts_with_ci(trimmed, "query") {
        let first = trimmed
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim_matches('{')
            .trim_matches('}');
        if !first.is_empty() && !first.contains('(') && !first.contains(')') {
            let parts: Vec<&str> = first.split(':').collect();
            if let Some(entity) = parts.last() {
                if !entity.is_empty() {
                    return Some((*entity).to_string());
                }
            }
        }
    }

    if let Some(idx) = trimmed.to_ascii_lowercase().rfind("type(") {
        let after = &trimmed[idx + "type(".len()..];
        if let Some(end) = after.find(')') {
            let entity = after[..end].split(':').next_back().unwrap_or("").trim();
            if !entity.is_empty() {
                return Some(entity.to_string());
            }
        }
    }

    if let Some(idx) = trimmed.to_ascii_lowercase().rfind("uid(") {
        let after = &trimmed[idx + "uid(".len()..];
        if let Some(end) = after.find(')') {
            let value = after[..end].trim();
            let parts: Vec<&str> = value.split(':').collect();
            if parts.len() >= 2 {
                let entity_idx = parts.len().saturating_sub(2);
                let entity = parts[entity_idx].trim();
                if !entity.is_empty() {
                    return Some(entity.to_string());
                }
            }
        }
    }

    None
}

fn lookup_case_insensitive<'a>(
    map: &'a HashMap<String, Vec<String>>,
    key: &str,
) -> Option<&'a Vec<String>> {
    map.iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(key))
        .map(|(_, value)| value)
}

fn highlight_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 16);
    let mut i = 0usize;
    while i < line.len() {
        let ch = line[i..].chars().next().unwrap_or_default();
        if is_token_char(ch) {
            let start = i;
            i += ch.len_utf8();
            while i < line.len() {
                let next = line[i..].chars().next().unwrap_or_default();
                if !is_token_char(next) {
                    break;
                }
                i += next.len_utf8();
            }
            let token = &line[start..i];
            if token.starts_with(':') {
                out.push_str(ANSI_META);
                out.push_str(token);
                out.push_str(ANSI_RESET);
            } else if token.starts_with('$') {
                out.push_str(ANSI_PARAM);
                out.push_str(token);
                out.push_str(ANSI_RESET);
            } else if token.starts_with('@') {
                out.push_str(ANSI_DIRECTIVE);
                out.push_str(token);
                out.push_str(ANSI_RESET);
            } else if is_sql_keyword(token) {
                out.push_str(ANSI_KEYWORD);
                out.push_str(token);
                out.push_str(ANSI_RESET);
            } else {
                out.push_str(token);
            }
        } else {
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

fn is_sql_keyword(token: &str) -> bool {
    SQL_KEYWORDS
        .iter()
        .any(|keyword| keyword.eq_ignore_ascii_case(token))
}

fn is_token_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$' | '@' | ':')
}

fn starts_with_ci(haystack: &str, prefix: &str) -> bool {
    haystack
        .get(..prefix.len())
        .map(|p| p.eq_ignore_ascii_case(prefix))
        .unwrap_or(false)
}

fn find_word_start(line: &str, pos: usize) -> usize {
    let upto = &line[..pos];
    let mut start = 0usize;
    for (idx, ch) in upto.char_indices().rev() {
        if ch.is_whitespace()
            || matches!(
                ch,
                '{' | '}' | '(' | ')' | '[' | ']' | ',' | ';' | ':' | '='
            )
        {
            start = idx + ch.len_utf8();
            break;
        }
    }
    start
}

fn dedupe_sorted(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut items: Vec<String> = values
        .into_iter()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect();
    items.sort_by_key(|v| v.to_ascii_lowercase());
    items.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyline::history::DefaultHistory;

    fn test_ctx() -> Context<'static> {
        let history = Box::leak(Box::new(DefaultHistory::new()));
        Context::new(history)
    }

    fn test_schema_catalog() -> ReplSchemaCatalog {
        let mut fields_by_entity = HashMap::new();
        fields_by_entity.insert(
            "Person".to_string(),
            vec!["name".to_string(), "age".to_string()],
        );
        let mut edges_by_entity = HashMap::new();
        edges_by_entity.insert("Person".to_string(), vec!["KNOWS".to_string()]);
        ReplSchemaCatalog {
            entity_types: vec!["Person".to_string(), "Company".to_string()],
            fields_by_entity,
            edges_by_entity,
        }
    }

    #[test]
    fn test_needs_continuation() {
        assert!(needs_continuation("Person {"));
        assert!(needs_continuation("query { start(func: type(Person)) {"));
        assert!(needs_continuation(
            "CREATE TYPE Person (name STRING REQUIRED)"
        ));
        assert!(needs_continuation(
            "Person @filter(name == \"alice\") { name"
        ));
        assert!(!needs_continuation("Person { name }"));
        assert!(!needs_continuation(
            "query { start(func: type(Person)) { name } }"
        ));
        assert!(!needs_continuation(
            "CREATE TYPE Person (name STRING REQUIRED);"
        ));
    }

    #[test]
    fn test_meta_completion() {
        let helper = ReplHelper::new();
        let (start, choices) = helper.complete(":fo", 3, &test_ctx()).unwrap();
        assert_eq!(start, 0);
        assert!(choices.iter().any(|c| c.replacement == ":format "));
    }

    #[test]
    fn test_set_completion_options() {
        let helper = ReplHelper::new();
        let line = ":set completion f";
        let (_start, choices) = helper.complete(line, line.len(), &test_ctx()).unwrap();
        assert!(choices.iter().any(|c| c.replacement == "fuzzy"));
    }

    #[test]
    fn test_sql_phrase_completion() {
        let helper = ReplHelper::new();
        let line = "SHOW REP";
        let (start, choices) = helper.complete(line, line.len(), &test_ctx()).unwrap();
        assert_eq!(start, 0);
        assert!(choices
            .iter()
            .any(|c| c.replacement == "SHOW REPLICATION STATUS"));
    }

    #[test]
    fn test_pql_func_and_directive_completion() {
        let helper = ReplHelper::new();

        let func_line = "query { result(func: t";
        let (_start, func_choices) = helper
            .complete(func_line, func_line.len(), &test_ctx())
            .unwrap();
        assert!(func_choices.iter().any(|c| c.replacement == "type("));

        let directive_line = "@filt";
        let (_start, dir_choices) = helper
            .complete(directive_line, directive_line.len(), &test_ctx())
            .unwrap();
        assert!(dir_choices.iter().any(|c| c.replacement == "@filter("));
    }

    #[test]
    fn test_schema_aware_field_and_edge_completion() {
        let mut helper = ReplHelper::new();
        helper.set_schema_catalog(&test_schema_catalog());

        let line = "Person { na";
        let (_start, field_choices) = helper.complete(line, line.len(), &test_ctx()).unwrap();
        assert!(field_choices.iter().any(|c| c.replacement == "name"));

        let edge_line = "Person { -[K";
        let (_start, edge_choices) = helper
            .complete(edge_line, edge_line.len(), &test_ctx())
            .unwrap();
        assert!(edge_choices.iter().any(|c| c.replacement == "KNOWS"));
    }

    #[test]
    fn test_fuzzy_completion_ranking_and_prefix_mode() {
        let mut helper = ReplHelper::new();
        helper.set_completion_mode(ReplCompletionMode::Fuzzy);

        let fuzzy_line = "@fitr";
        let (_start, fuzzy_choices) = helper
            .complete(fuzzy_line, fuzzy_line.len(), &test_ctx())
            .unwrap();
        assert_eq!(
            fuzzy_choices.first().map(|c| c.replacement.as_str()),
            Some("@filter(")
        );

        helper.set_completion_mode(ReplCompletionMode::Prefix);
        let (_start, prefix_choices) = helper
            .complete(fuzzy_line, fuzzy_line.len(), &test_ctx())
            .unwrap();
        assert!(!prefix_choices.iter().any(|c| c.replacement == "@filter("));
    }

    #[test]
    fn test_color_off_disables_ansi() {
        let mut helper = ReplHelper::new();
        helper.set_color_mode(ReplColorMode::Off);

        assert_eq!(
            helper.highlight("SHOW SCHEMAS", 12).into_owned(),
            "SHOW SCHEMAS"
        );
        assert_eq!(
            helper
                .highlight_prompt("default:default> ", false)
                .into_owned(),
            "default:default> "
        );
        assert_eq!(helper.highlight_hint(" hint").into_owned(), " hint");
        assert_eq!(
            helper
                .highlight_candidate("candidate", rustyline::CompletionType::List)
                .into_owned(),
            "candidate"
        );
    }
}
