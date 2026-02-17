use anyhow::{Context, Result};
use axum::extract::{Path, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Router;
use pulldown_cmark::{html, Options, Parser};
use std::fs;
use std::path::{Path as FsPath, PathBuf};
use tokio::sync::watch;
use tracing::info;

#[derive(Clone)]
struct DocsState {
    docs_dir: PathBuf,
    title: String,
}

#[derive(Clone)]
struct DocEntry {
    slug: String,
    file_name: String,
    title: String,
}

pub async fn serve_docs(
    addr: &str,
    docs_dir: PathBuf,
    title: String,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind docs address {addr}"))?;

    let state = DocsState { docs_dir, title };
    let app = Router::new()
        .route("/", get(root_redirect))
        .route("/docs", get(root_redirect))
        .route("/docs/", get(index_handler))
        .route("/docs/:slug", get(page_handler))
        .route("/docs/assets/docs.css", get(css_handler))
        .with_state(state);

    info!("Docs site enabled at http://{}/docs/", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            loop {
                if *shutdown.borrow() {
                    break;
                }
                if shutdown.changed().await.is_err() {
                    break;
                }
            }
        })
        .await
        .context("docs server failed")?;

    Ok(())
}

async fn root_redirect() -> Redirect {
    Redirect::permanent("/docs/")
}

async fn css_handler() -> impl IntoResponse {
    ([(CONTENT_TYPE, "text/css; charset=utf-8")], DOCS_CSS)
}

async fn index_handler(State(state): State<DocsState>) -> Response {
    match discover_docs(&state.docs_dir) {
        Ok(docs) => {
            let mut items = String::new();
            for doc in docs {
                items.push_str(&format!(
                    "<li><a href=\"/docs/{}\">{}</a><span class=\"filename\">{}</span></li>",
                    escape_html(&doc.slug),
                    escape_html(&doc.title),
                    escape_html(&doc.file_name)
                ));
            }

            Html(layout(
                &state.title,
                "Documentation",
                &format!(
                    "<section class=\"panel\"><h2>Documentation</h2><p>PelagoDB docs are rendered from local markdown files.</p><ul class=\"doc-list\">{items}</ul></section>"
                ),
            ))
            .into_response()
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Html(layout(
                &state.title,
                "Documentation Error",
                &format!("<section class=\"panel error\"><h2>Failed to load docs</h2><pre>{}</pre></section>", escape_html(&err.to_string())),
            )),
        )
            .into_response(),
    }
}

async fn page_handler(
    Path(slug): Path<String>,
    State(state): State<DocsState>,
) -> impl IntoResponse {
    if !is_valid_slug(&slug) {
        return (
            StatusCode::BAD_REQUEST,
            Html(layout(
                &state.title,
                "Invalid Page",
                "<section class=\"panel error\"><h2>Invalid docs page slug</h2></section>",
            )),
        )
            .into_response();
    }

    let path = state.docs_dir.join(format!("{slug}.md"));
    match fs::read_to_string(&path) {
        Ok(markdown) => {
            let body_html = render_markdown(&markdown);
            let page_title = extract_title(&markdown).unwrap_or_else(|| slug_to_title(&slug));
            let html = layout(
                &state.title,
                &page_title,
                &format!(
                    "<article class=\"doc-content\">{}</article><p class=\"back-link\"><a href=\"/docs/\">Back to index</a></p>",
                    body_html
                ),
            );
            (StatusCode::OK, Html(html)).into_response()
        }
        Err(_) => (
            StatusCode::NOT_FOUND,
            Html(layout(
                &state.title,
                "Page Not Found",
                &format!(
                    "<section class=\"panel error\"><h2>Page not found</h2><p>No markdown file found for <code>{}</code>.</p></section>",
                    escape_html(&slug)
                ),
            )),
        )
            .into_response(),
    }
}

fn discover_docs(docs_dir: &FsPath) -> Result<Vec<DocEntry>> {
    let mut docs = Vec::new();

    for entry in fs::read_dir(docs_dir)
        .with_context(|| format!("failed to read docs dir: {}", docs_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }

        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(stem) => stem.to_string(),
            None => continue,
        };

        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();

        let markdown = fs::read_to_string(&path).unwrap_or_default();
        let title = extract_title(&markdown).unwrap_or_else(|| slug_to_title(&stem));

        docs.push(DocEntry {
            slug: stem,
            file_name,
            title,
        });
    }

    docs.sort_by(|a, b| {
        if a.slug == "README" {
            std::cmp::Ordering::Less
        } else if b.slug == "README" {
            std::cmp::Ordering::Greater
        } else {
            a.slug.cmp(&b.slug)
        }
    });

    Ok(docs)
}

fn extract_title(markdown: &str) -> Option<String> {
    for line in markdown.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("# ") {
            let title = rest.trim();
            if !title.is_empty() {
                return Some(title.to_string());
            }
        }
    }
    None
}

fn slug_to_title(slug: &str) -> String {
    slug.replace('-', " ").replace('_', " ")
}

fn is_valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn render_markdown(markdown: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(markdown, options);
    let mut html_out = String::new();
    html::push_html(&mut html_out, parser);
    html_out
}

fn layout(site_title: &str, page_title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{} | {}</title><link rel=\"stylesheet\" href=\"/docs/assets/docs.css\"></head><body><main class=\"container\"><header><h1>{}</h1><p class=\"subtitle\">{}</p></header>{}</main></body></html>",
        escape_html(page_title),
        escape_html(site_title),
        escape_html(site_title),
        escape_html(page_title),
        body
    )
}

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const DOCS_CSS: &str = r#"
:root {
  --bg: #f4efe6;
  --panel: #fffdf9;
  --text: #1b1c1f;
  --muted: #55606a;
  --accent: #0d6e70;
  --border: #ded5c8;
}
* { box-sizing: border-box; }
body {
  margin: 0;
  font-family: Georgia, "Iowan Old Style", "Palatino Linotype", serif;
  color: var(--text);
  background: radial-gradient(circle at 20% -10%, #faf4ea 0, var(--bg) 50%, #ede6db 100%);
}
.container {
  max-width: 980px;
  margin: 0 auto;
  padding: 2rem 1rem 4rem;
}
header {
  margin-bottom: 1rem;
}
h1 {
  margin: 0;
  font-size: clamp(1.8rem, 4vw, 2.8rem);
  letter-spacing: 0.02em;
}
.subtitle {
  margin: 0.3rem 0 0;
  color: var(--muted);
}
.panel {
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: 14px;
  padding: 1rem 1.1rem;
}
.panel.error {
  border-color: #c07474;
  background: #fff5f5;
}
.doc-list {
  list-style: none;
  padding: 0;
  margin: 1rem 0 0;
}
.doc-list li {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem;
  justify-content: space-between;
  align-items: baseline;
  padding: 0.55rem 0;
  border-bottom: 1px solid var(--border);
}
.doc-list li:last-child { border-bottom: none; }
.doc-list a {
  color: var(--accent);
  text-decoration: none;
  font-weight: 600;
}
.doc-list a:hover { text-decoration: underline; }
.filename {
  color: var(--muted);
  font-size: 0.9rem;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
}
.doc-content {
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: 14px;
  padding: 1.25rem;
  line-height: 1.65;
}
.doc-content h1, .doc-content h2, .doc-content h3 {
  margin-top: 1.2em;
  margin-bottom: 0.45em;
  line-height: 1.3;
}
.doc-content pre {
  overflow-x: auto;
  border-radius: 10px;
  border: 1px solid var(--border);
  background: #f8f4ed;
  padding: 0.8rem;
}
.doc-content code {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 0.92em;
}
.doc-content table {
  width: 100%;
  border-collapse: collapse;
  margin: 1rem 0;
}
.doc-content th, .doc-content td {
  border: 1px solid var(--border);
  padding: 0.45rem 0.5rem;
}
.back-link {
  margin-top: 0.8rem;
}
.back-link a {
  color: var(--accent);
  text-decoration: none;
}
.back-link a:hover {
  text-decoration: underline;
}
@media (max-width: 640px) {
  .doc-content { padding: 0.95rem; }
}
"#;

#[cfg(test)]
mod tests {
    use super::{extract_title, is_valid_slug};

    #[test]
    fn title_extraction_prefers_h1() {
        let md = "# Hello\n\ncontent";
        assert_eq!(extract_title(md).as_deref(), Some("Hello"));
    }

    #[test]
    fn slug_validation_rejects_path_chars() {
        assert!(is_valid_slug("01-getting-started"));
        assert!(!is_valid_slug("../../etc/passwd"));
        assert!(!is_valid_slug("has space"));
    }
}
