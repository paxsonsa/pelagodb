use comfy_table::{ContentArrangement, Table};

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
    Csv,
}

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputFormat::Table => write!(f, "table"),
            OutputFormat::Json => write!(f, "json"),
            OutputFormat::Csv => write!(f, "csv"),
        }
    }
}

pub fn print_table(headers: &[&str], rows: &[Vec<String>]) {
    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.set_header(headers);
    for row in rows {
        table.add_row(row);
    }
    println!("{table}");
}

pub fn print_json(value: &serde_json::Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_default()
    );
}

pub fn print_csv(headers: &[&str], rows: &[Vec<String>]) {
    println!("{}", headers.join(","));
    for row in rows {
        println!("{}", row.join(","));
    }
}

pub fn format_output(
    headers: &[&str],
    rows: &[Vec<String>],
    json_value: &serde_json::Value,
    format: &OutputFormat,
) {
    match format {
        OutputFormat::Table => print_table(headers, rows),
        OutputFormat::Json => print_json(json_value),
        OutputFormat::Csv => print_csv(headers, rows),
    }
}
