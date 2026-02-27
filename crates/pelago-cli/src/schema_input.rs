use pelago_proto::{
    edge_target, value, EdgeDef, EdgeDirectionDef, EdgeTarget, EntitySchema, ExtrasPolicy,
    IndexType, OwnershipMode, PropertyDef, PropertyType, SchemaMeta, Value,
};
use std::collections::HashMap;

#[derive(clap::ValueEnum, Debug, Clone, Copy, Default)]
pub enum SchemaInputFormat {
    #[default]
    Auto,
    Json,
    Sql,
}

pub fn parse_schema_input(input: &str, format: SchemaInputFormat) -> Result<EntitySchema, String> {
    match format {
        SchemaInputFormat::Json => parse_schema_json_str(input),
        SchemaInputFormat::Sql => parse_schema_sql(input),
        SchemaInputFormat::Auto => {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(input) {
                parse_schema_json_value(&json)
            } else {
                parse_schema_sql(input)
            }
        }
    }
}

pub fn parse_schema_json_str(input: &str) -> Result<EntitySchema, String> {
    let json: serde_json::Value = serde_json::from_str(input)
        .map_err(|e| format!("Failed to parse schema JSON payload: {}", e))?;
    parse_schema_json_value(&json)
}

pub fn parse_schema_json_value(json: &serde_json::Value) -> Result<EntitySchema, String> {
    let obj = json
        .as_object()
        .ok_or_else(|| "Schema JSON must be an object".to_string())?;

    let name = get_string_field(obj, &["name", "entity_type", "entityType"])
        .ok_or_else(|| "Missing required schema field: name".to_string())?;

    let mut properties = HashMap::new();
    if let Some(prop_obj) = obj.get("properties").and_then(|v| v.as_object()) {
        for (prop_name, prop_value) in prop_obj {
            properties.insert(
                prop_name.clone(),
                parse_property_def_json(prop_name, prop_value)?,
            );
        }
    }

    let mut edges = HashMap::new();
    if let Some(edge_obj) = obj.get("edges").and_then(|v| v.as_object()) {
        for (edge_name, edge_value) in edge_obj {
            edges.insert(
                edge_name.clone(),
                parse_edge_def_json(edge_name, edge_value)?,
            );
        }
    }

    let meta = obj
        .get("meta")
        .or_else(|| obj.get("schema_meta"))
        .or_else(|| obj.get("schemaMeta"))
        .and_then(|v| v.as_object())
        .map(parse_schema_meta_json)
        .transpose()?;

    Ok(EntitySchema {
        name,
        version: 0,
        properties,
        edges,
        meta,
        created_at: get_i64_field(obj, &["created_at", "createdAt"]).unwrap_or_default(),
        created_by: get_string_field(obj, &["created_by", "createdBy"]).unwrap_or_default(),
    })
}

fn parse_schema_meta_json(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<SchemaMeta, String> {
    let allow_undeclared_edges =
        get_bool_field(obj, &["allow_undeclared_edges", "allowUndeclaredEdges"]).unwrap_or(false);
    let extras_policy = match get_value_field(obj, &["extras_policy", "extrasPolicy"]) {
        Some(value) => parse_extras_policy_value(value)?,
        None => ExtrasPolicy::Unspecified,
    };

    Ok(SchemaMeta {
        allow_undeclared_edges,
        extras_policy: extras_policy as i32,
    })
}

fn parse_property_def_json(name: &str, value: &serde_json::Value) -> Result<PropertyDef, String> {
    let obj = value
        .as_object()
        .ok_or_else(|| format!("Property '{}' must be an object", name))?;

    let prop_type_value = get_value_field(obj, &["type", "property_type", "propertyType"])
        .ok_or_else(|| format!("Property '{}' missing required field: type", name))?;
    let prop_type = parse_property_type_value(prop_type_value)?;

    let required = get_bool_field(obj, &["required", "not_null", "notNull"]).unwrap_or(false);

    let index = match get_value_field(obj, &["index"]) {
        Some(v) => Some(parse_index_type_value(v)? as i32),
        None => None,
    };

    let default_value = match get_value_field(obj, &["default_value", "defaultValue", "default"]) {
        Some(v) => Some(parse_proto_value(v)?),
        None => None,
    };

    Ok(PropertyDef {
        r#type: prop_type as i32,
        required,
        index,
        default_value,
    })
}

fn parse_edge_def_json(name: &str, value: &serde_json::Value) -> Result<EdgeDef, String> {
    if let Some(target_name) = value.as_str() {
        return Ok(EdgeDef {
            target: Some(parse_target_from_string(target_name)),
            direction: EdgeDirectionDef::Outgoing as i32,
            properties: HashMap::new(),
            sort_key: String::new(),
            ownership: OwnershipMode::SourceSite as i32,
        });
    }

    let obj = value
        .as_object()
        .ok_or_else(|| format!("Edge '{}' must be an object or target string", name))?;

    let target_value = get_value_field(obj, &["target"])
        .ok_or_else(|| format!("Edge '{}' missing required field: target", name))?;
    let target = parse_edge_target_value(target_value)?;

    let direction = match get_value_field(obj, &["direction"]) {
        Some(v) => parse_edge_direction_value(v)?,
        None => EdgeDirectionDef::Outgoing,
    };

    let ownership = match get_value_field(obj, &["ownership"]) {
        Some(v) => parse_ownership_mode_value(v)?,
        None => OwnershipMode::SourceSite,
    };

    let sort_key = get_string_field(obj, &["sort_key", "sortKey"]).unwrap_or_default();

    let mut properties = HashMap::new();
    if let Some(props_obj) = obj.get("properties").and_then(|v| v.as_object()) {
        for (prop_name, prop_value) in props_obj {
            properties.insert(
                prop_name.clone(),
                parse_property_def_json(prop_name, prop_value)?,
            );
        }
    }

    Ok(EdgeDef {
        target: Some(target),
        direction: direction as i32,
        properties,
        sort_key,
        ownership: ownership as i32,
    })
}

fn parse_edge_target_value(value: &serde_json::Value) -> Result<EdgeTarget, String> {
    if let Some(target_name) = value.as_str() {
        return Ok(parse_target_from_string(target_name));
    }

    let obj = value
        .as_object()
        .ok_or_else(|| "Edge target must be a string or object".to_string())?;

    if let Some(specific) = get_string_field(obj, &["specific_type", "specificType"]) {
        return Ok(EdgeTarget {
            kind: Some(edge_target::Kind::SpecificType(specific)),
        });
    }

    if let Some(true) = get_bool_field(obj, &["polymorphic"]) {
        return Ok(EdgeTarget {
            kind: Some(edge_target::Kind::Polymorphic(true)),
        });
    }

    if let Some(kind_obj) = obj.get("kind").and_then(|v| v.as_object()) {
        if let Some(specific) = get_string_field(kind_obj, &["specific_type", "specificType"]) {
            return Ok(EdgeTarget {
                kind: Some(edge_target::Kind::SpecificType(specific)),
            });
        }
        if let Some(true) = get_bool_field(kind_obj, &["polymorphic"]) {
            return Ok(EdgeTarget {
                kind: Some(edge_target::Kind::Polymorphic(true)),
            });
        }
    }

    Err("Edge target must include specific_type or polymorphic=true".to_string())
}

fn parse_target_from_string(target: &str) -> EdgeTarget {
    let target = target.trim();
    if target == "*" || target.eq_ignore_ascii_case("polymorphic") {
        EdgeTarget {
            kind: Some(edge_target::Kind::Polymorphic(true)),
        }
    } else {
        EdgeTarget {
            kind: Some(edge_target::Kind::SpecificType(target.to_string())),
        }
    }
}

fn parse_property_type_value(value: &serde_json::Value) -> Result<PropertyType, String> {
    if let Some(code) = value.as_i64() {
        return PropertyType::try_from(code as i32)
            .map_err(|_| format!("Unknown property type enum value: {}", code));
    }

    let text = value
        .as_str()
        .ok_or_else(|| "Property type must be a string or enum value".to_string())?;

    let normalized = normalize_token(text);
    let ty = match normalized.as_str() {
        "string" | "str" | "property_type_string" => PropertyType::String,
        "int" | "integer" | "i64" | "property_type_int" => PropertyType::Int,
        "float" | "double" | "f64" | "property_type_float" => PropertyType::Float,
        "bool" | "boolean" | "property_type_bool" => PropertyType::Bool,
        "timestamp" | "time" | "property_type_timestamp" => PropertyType::Timestamp,
        "bytes" | "binary" | "property_type_bytes" => PropertyType::Bytes,
        _ => {
            return Err(format!(
                "Unsupported property type '{}'. Expected one of: string,int,float,bool,timestamp,bytes",
                text
            ));
        }
    };

    Ok(ty)
}

fn parse_index_type_value(value: &serde_json::Value) -> Result<IndexType, String> {
    if let Some(code) = value.as_i64() {
        return IndexType::try_from(code as i32)
            .map_err(|_| format!("Unknown index type enum value: {}", code));
    }

    let text = value
        .as_str()
        .ok_or_else(|| "Index type must be a string or enum value".to_string())?;
    let normalized = normalize_token(text);

    let index = match normalized.as_str() {
        "none" | "index_type_none" => IndexType::None,
        "unique" | "index_type_unique" => IndexType::Unique,
        "equality" | "eq" | "index_type_equality" => IndexType::Equality,
        "range" | "index_type_range" => IndexType::Range,
        _ => {
            return Err(format!(
                "Unsupported index type '{}'. Expected one of: none,unique,equality,range",
                text
            ));
        }
    };

    Ok(index)
}

fn parse_extras_policy_value(value: &serde_json::Value) -> Result<ExtrasPolicy, String> {
    if let Some(code) = value.as_i64() {
        return ExtrasPolicy::try_from(code as i32)
            .map_err(|_| format!("Unknown extras policy enum value: {}", code));
    }

    let text = value
        .as_str()
        .ok_or_else(|| "extras_policy must be a string or enum value".to_string())?;
    let normalized = normalize_token(text);

    let policy = match normalized.as_str() {
        "reject" | "extras_policy_reject" => ExtrasPolicy::Reject,
        "allow" | "extras_policy_allow" => ExtrasPolicy::Allow,
        "warn" | "extras_policy_warn" => ExtrasPolicy::Warn,
        "unspecified" | "extras_policy_unspecified" => ExtrasPolicy::Unspecified,
        _ => {
            return Err(format!(
                "Unsupported extras policy '{}'. Expected one of: reject,allow,warn",
                text
            ));
        }
    };

    Ok(policy)
}

fn parse_edge_direction_value(value: &serde_json::Value) -> Result<EdgeDirectionDef, String> {
    if let Some(code) = value.as_i64() {
        return EdgeDirectionDef::try_from(code as i32)
            .map_err(|_| format!("Unknown edge direction enum value: {}", code));
    }

    let text = value
        .as_str()
        .ok_or_else(|| "Edge direction must be a string or enum value".to_string())?;
    let normalized = normalize_token(text);

    let direction = match normalized.as_str() {
        "outgoing" | "edge_direction_def_outgoing" | "edge_direction_outgoing" => {
            EdgeDirectionDef::Outgoing
        }
        "bidirectional" | "edge_direction_def_bidirectional" | "edge_direction_bidirectional" => {
            EdgeDirectionDef::Bidirectional
        }
        _ => {
            return Err(format!(
                "Unsupported edge direction '{}'. Expected outgoing|bidirectional",
                text
            ));
        }
    };

    Ok(direction)
}

fn parse_ownership_mode_value(value: &serde_json::Value) -> Result<OwnershipMode, String> {
    if let Some(code) = value.as_i64() {
        return OwnershipMode::try_from(code as i32)
            .map_err(|_| format!("Unknown ownership mode enum value: {}", code));
    }

    let text = value
        .as_str()
        .ok_or_else(|| "Ownership mode must be a string or enum value".to_string())?;
    let normalized = normalize_token(text);

    let ownership = match normalized.as_str() {
        "source_site" | "sourcesite" | "ownership_mode_source_site" => OwnershipMode::SourceSite,
        "independent" | "ownership_mode_independent" => OwnershipMode::Independent,
        _ => {
            return Err(format!(
                "Unsupported ownership mode '{}'. Expected source_site|independent",
                text
            ));
        }
    };

    Ok(ownership)
}

fn parse_proto_value(value: &serde_json::Value) -> Result<Value, String> {
    if let Some(obj) = value.as_object() {
        if obj.len() == 1 {
            if let Some(v) = obj.get("string_value").or_else(|| obj.get("stringValue")) {
                return Ok(Value {
                    kind: Some(value::Kind::StringValue(
                        v.as_str()
                            .ok_or_else(|| "string_value must be a string".to_string())?
                            .to_string(),
                    )),
                });
            }
            if let Some(v) = obj.get("int_value").or_else(|| obj.get("intValue")) {
                return Ok(Value {
                    kind: Some(value::Kind::IntValue(
                        v.as_i64()
                            .ok_or_else(|| "int_value must be an integer".to_string())?,
                    )),
                });
            }
            if let Some(v) = obj.get("float_value").or_else(|| obj.get("floatValue")) {
                return Ok(Value {
                    kind: Some(value::Kind::FloatValue(
                        v.as_f64()
                            .ok_or_else(|| "float_value must be a number".to_string())?,
                    )),
                });
            }
            if let Some(v) = obj.get("bool_value").or_else(|| obj.get("boolValue")) {
                return Ok(Value {
                    kind: Some(value::Kind::BoolValue(
                        v.as_bool()
                            .ok_or_else(|| "bool_value must be true/false".to_string())?,
                    )),
                });
            }
            if let Some(v) = obj
                .get("timestamp_value")
                .or_else(|| obj.get("timestampValue"))
            {
                return Ok(Value {
                    kind: Some(value::Kind::TimestampValue(v.as_i64().ok_or_else(
                        || "timestamp_value must be an integer (unix micros)".to_string(),
                    )?)),
                });
            }
            if let Some(v) = obj.get("bytes_value").or_else(|| obj.get("bytesValue")) {
                return Ok(Value {
                    kind: Some(value::Kind::BytesValue(
                        v.as_str()
                            .ok_or_else(|| "bytes_value must be a string".to_string())?
                            .as_bytes()
                            .to_vec(),
                    )),
                });
            }
            if let Some(v) = obj.get("null_value").or_else(|| obj.get("nullValue")) {
                return Ok(Value {
                    kind: Some(value::Kind::NullValue(v.as_bool().unwrap_or(true))),
                });
            }
        }
    }

    Ok(match value {
        serde_json::Value::String(s) => Value {
            kind: Some(value::Kind::StringValue(s.clone())),
        },
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value {
                    kind: Some(value::Kind::IntValue(i)),
                }
            } else {
                Value {
                    kind: Some(value::Kind::FloatValue(n.as_f64().unwrap_or_default())),
                }
            }
        }
        serde_json::Value::Bool(b) => Value {
            kind: Some(value::Kind::BoolValue(*b)),
        },
        serde_json::Value::Null => Value {
            kind: Some(value::Kind::NullValue(true)),
        },
        _ => Value {
            kind: Some(value::Kind::StringValue(value.to_string())),
        },
    })
}

fn normalize_token(token: &str) -> String {
    token
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .replace(' ', "_")
}

fn get_value_field<'a>(
    obj: &'a serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<&'a serde_json::Value> {
    keys.iter().find_map(|key| obj.get(*key))
}

fn get_string_field(
    obj: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<String> {
    get_value_field(obj, keys).and_then(|v| v.as_str().map(|s| s.to_string()))
}

fn get_bool_field(obj: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> Option<bool> {
    get_value_field(obj, keys).and_then(|v| v.as_bool())
}

fn get_i64_field(obj: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> Option<i64> {
    get_value_field(obj, keys).and_then(|v| v.as_i64())
}

#[derive(Clone, Debug)]
struct SqlToken {
    text: String,
    quoted: bool,
}

pub fn parse_schema_sql(input: &str) -> Result<EntitySchema, String> {
    let tokens = tokenize_sql(input)?;
    let mut parser = SqlParser::new(tokens);
    let schema = parser.parse_schema()?;
    parser.consume_semicolons();
    if !parser.is_eof() {
        return Err(format!(
            "Unexpected token '{}' after end of CREATE TYPE definition",
            parser.peek().map(|t| t.text.as_str()).unwrap_or_default()
        ));
    }
    Ok(schema)
}

struct SqlParser {
    tokens: Vec<SqlToken>,
    pos: usize,
}

impl SqlParser {
    fn new(tokens: Vec<SqlToken>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn parse_schema(&mut self) -> Result<EntitySchema, String> {
        self.expect_keyword("create")?;
        if !self.match_keyword("type") && !self.match_keyword("schema") {
            return Err("Expected TYPE or SCHEMA after CREATE".to_string());
        }

        let name = self.expect_identifier("schema name")?;

        self.expect_symbol("(")?;
        let properties = self.parse_property_list("schema properties")?;
        self.expect_symbol(")")?;

        let mut schema = EntitySchema {
            name,
            version: 0,
            properties,
            edges: HashMap::new(),
            meta: None,
            created_at: 0,
            created_by: String::new(),
        };

        loop {
            self.consume_semicolons();
            if self.is_eof() {
                break;
            }

            if self.match_keyword("with") {
                schema.meta = Some(self.parse_with_clause()?);
                continue;
            }

            if self.match_keyword("edge") {
                let (label, edge) = self.parse_edge_clause()?;
                schema.edges.insert(label, edge);
                continue;
            }

            break;
        }

        Ok(schema)
    }

    fn parse_with_clause(&mut self) -> Result<SchemaMeta, String> {
        self.expect_symbol("(")?;

        let mut allow_undeclared_edges = false;
        let mut extras_policy = ExtrasPolicy::Unspecified;

        while !self.check_symbol(")") {
            let key = normalize_token(&self.expect_identifier("WITH option key")?);
            self.expect_symbol("=")?;

            match key.as_str() {
                "allow_undeclared_edges" => {
                    allow_undeclared_edges = self.parse_bool_literal("allow_undeclared_edges")?;
                }
                "extras_policy" => {
                    let value = self
                        .next_token()
                        .ok_or_else(|| "Missing extras_policy value in WITH clause".to_string())?;
                    extras_policy = parse_extras_policy_token(&value.text)?;
                }
                _ => return Err(format!("Unsupported WITH option '{}'", key)),
            }

            if self.match_symbol(",") {
                continue;
            }
            if self.check_symbol(")") {
                break;
            }
            return Err("Expected ',' or ')' after WITH option".to_string());
        }

        self.expect_symbol(")")?;

        Ok(SchemaMeta {
            allow_undeclared_edges,
            extras_policy: extras_policy as i32,
        })
    }

    fn parse_edge_clause(&mut self) -> Result<(String, EdgeDef), String> {
        let label = self.expect_identifier("edge label")?;

        if !self.match_keyword("to") && !self.match_keyword("target") {
            return Err("EDGE clause requires TO <target>".to_string());
        }

        let target_token = self
            .next_token()
            .ok_or_else(|| "Missing edge target after TO".to_string())?;
        let target = parse_target_from_string(&target_token.text);

        let mut edge = EdgeDef {
            target: Some(target),
            direction: EdgeDirectionDef::Outgoing as i32,
            properties: HashMap::new(),
            sort_key: String::new(),
            ownership: OwnershipMode::SourceSite as i32,
        };

        loop {
            if self.is_eof()
                || self.check_symbol(";")
                || self.check_keyword("edge")
                || self.check_keyword("with")
            {
                break;
            }

            if self.check_symbol("(") {
                self.expect_symbol("(")?;
                edge.properties = self.parse_property_list("edge properties")?;
                self.expect_symbol(")")?;
                continue;
            }

            if self.match_keyword("direction") {
                let tok = self
                    .next_token()
                    .ok_or_else(|| "Missing edge direction value".to_string())?;
                edge.direction = parse_edge_direction_token(&tok.text)? as i32;
                continue;
            }

            if self.match_keyword("ownership") {
                let tok = self
                    .next_token()
                    .ok_or_else(|| "Missing edge ownership value".to_string())?;
                edge.ownership = parse_ownership_mode_token(&tok.text)? as i32;
                continue;
            }

            if self.match_keyword("sort_key") || self.match_compound_keyword("sort", "key") {
                edge.sort_key = self.expect_identifier("sort key property")?;
                continue;
            }

            let token = self
                .peek()
                .map(|t| t.text.clone())
                .unwrap_or_else(|| "<eof>".to_string());
            return Err(format!("Unexpected token '{}' in EDGE clause", token));
        }

        Ok((label, edge))
    }

    fn parse_property_list(&mut self, scope: &str) -> Result<HashMap<String, PropertyDef>, String> {
        let mut properties = HashMap::new();

        while !self.check_symbol(")") {
            let (name, def) = self.parse_property(scope)?;
            properties.insert(name, def);

            if self.match_symbol(",") {
                continue;
            }
            if self.check_symbol(")") {
                break;
            }

            return Err(format!("Expected ',' or ')' while parsing {}", scope));
        }

        Ok(properties)
    }

    fn parse_property(&mut self, scope: &str) -> Result<(String, PropertyDef), String> {
        let name = self.expect_identifier("property name")?;
        let type_token = self
            .next_token()
            .ok_or_else(|| format!("Missing type for property '{}' in {}", name, scope))?;
        let prop_type = parse_property_type_token(&type_token.text)?;

        let mut required = false;
        let mut index = None;
        let mut default_value = None;

        while !self.is_eof() && !self.check_symbol(",") && !self.check_symbol(")") {
            if self.match_keyword("required") {
                required = true;
                continue;
            }

            if self.match_compound_keyword("not", "null") {
                required = true;
                continue;
            }

            if self.match_keyword("index") {
                let token = self
                    .next_token()
                    .ok_or_else(|| format!("Missing index value for property '{}'", name))?;
                index = Some(parse_index_type_token(&token.text)? as i32);
                continue;
            }

            if self.match_keyword("default") {
                default_value = Some(self.parse_default_literal()?);
                continue;
            }

            let token = self
                .peek()
                .map(|t| t.text.clone())
                .unwrap_or_else(|| "<eof>".to_string());
            return Err(format!(
                "Unexpected token '{}' while parsing property '{}' in {}",
                token, name, scope
            ));
        }

        Ok((
            name,
            PropertyDef {
                r#type: prop_type as i32,
                required,
                index,
                default_value,
            },
        ))
    }

    fn parse_default_literal(&mut self) -> Result<Value, String> {
        let token = self
            .next_token()
            .ok_or_else(|| "Missing value after DEFAULT".to_string())?;

        if token.quoted {
            return Ok(Value {
                kind: Some(value::Kind::StringValue(token.text)),
            });
        }

        let normalized = normalize_token(&token.text);
        if normalized == "true" {
            return Ok(Value {
                kind: Some(value::Kind::BoolValue(true)),
            });
        }
        if normalized == "false" {
            return Ok(Value {
                kind: Some(value::Kind::BoolValue(false)),
            });
        }
        if normalized == "null" {
            return Ok(Value {
                kind: Some(value::Kind::NullValue(true)),
            });
        }

        if let Ok(i) = token.text.parse::<i64>() {
            return Ok(Value {
                kind: Some(value::Kind::IntValue(i)),
            });
        }

        if let Ok(f) = token.text.parse::<f64>() {
            return Ok(Value {
                kind: Some(value::Kind::FloatValue(f)),
            });
        }

        Ok(Value {
            kind: Some(value::Kind::StringValue(token.text)),
        })
    }

    fn parse_bool_literal(&mut self, field_name: &str) -> Result<bool, String> {
        let tok = self
            .next_token()
            .ok_or_else(|| format!("Missing boolean value for {}", field_name))?;
        match normalize_token(&tok.text).as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(format!(
                "{} expects true|false but got '{}'",
                field_name, tok.text
            )),
        }
    }

    fn consume_semicolons(&mut self) {
        while self.match_symbol(";") {}
    }

    fn match_compound_keyword(&mut self, a: &str, b: &str) -> bool {
        let start = self.pos;
        if self.match_keyword(a) && self.match_keyword(b) {
            true
        } else {
            self.pos = start;
            false
        }
    }

    fn expect_keyword(&mut self, expected: &str) -> Result<(), String> {
        if self.match_keyword(expected) {
            Ok(())
        } else {
            let got = self.peek().map(|t| t.text.as_str()).unwrap_or("<eof>");
            Err(format!(
                "Expected keyword '{}' but found '{}'",
                expected, got
            ))
        }
    }

    fn check_keyword(&self, keyword: &str) -> bool {
        self.peek()
            .map(|t| !t.quoted && t.text.eq_ignore_ascii_case(keyword))
            .unwrap_or(false)
    }

    fn match_keyword(&mut self, keyword: &str) -> bool {
        if self.check_keyword(keyword) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect_symbol(&mut self, symbol: &str) -> Result<(), String> {
        if self.match_symbol(symbol) {
            Ok(())
        } else {
            let got = self.peek().map(|t| t.text.as_str()).unwrap_or("<eof>");
            Err(format!("Expected symbol '{}' but found '{}'", symbol, got))
        }
    }

    fn check_symbol(&self, symbol: &str) -> bool {
        self.peek()
            .map(|t| !t.quoted && t.text == symbol)
            .unwrap_or(false)
    }

    fn match_symbol(&mut self, symbol: &str) -> bool {
        if self.check_symbol(symbol) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect_identifier(&mut self, context: &str) -> Result<String, String> {
        let token = self
            .next_token()
            .ok_or_else(|| format!("Expected {} but found end of input", context))?;

        if !token.quoted && matches!(token.text.as_str(), "(" | ")" | "," | ";" | "=") {
            return Err(format!("Expected {} but found '{}'", context, token.text));
        }

        Ok(token.text)
    }

    fn next_token(&mut self) -> Option<SqlToken> {
        let out = self.tokens.get(self.pos).cloned();
        if out.is_some() {
            self.pos += 1;
        }
        out
    }

    fn peek(&self) -> Option<&SqlToken> {
        self.tokens.get(self.pos)
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.tokens.len()
    }
}

fn parse_property_type_token(token: &str) -> Result<PropertyType, String> {
    parse_property_type_value(&serde_json::Value::String(token.to_string()))
}

fn parse_index_type_token(token: &str) -> Result<IndexType, String> {
    parse_index_type_value(&serde_json::Value::String(token.to_string()))
}

fn parse_extras_policy_token(token: &str) -> Result<ExtrasPolicy, String> {
    parse_extras_policy_value(&serde_json::Value::String(token.to_string()))
}

fn parse_edge_direction_token(token: &str) -> Result<EdgeDirectionDef, String> {
    parse_edge_direction_value(&serde_json::Value::String(token.to_string()))
}

fn parse_ownership_mode_token(token: &str) -> Result<OwnershipMode, String> {
    parse_ownership_mode_value(&serde_json::Value::String(token.to_string()))
}

fn tokenize_sql(input: &str) -> Result<Vec<SqlToken>, String> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch.is_whitespace() {
            continue;
        }

        if ch == '-' {
            if let Some('-') = chars.peek().copied() {
                chars.next();
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
                continue;
            }
        }

        if ch == '#' {
            for c in chars.by_ref() {
                if c == '\n' {
                    break;
                }
            }
            continue;
        }

        if matches!(ch, '(' | ')' | ',' | ';' | '=') {
            tokens.push(SqlToken {
                text: ch.to_string(),
                quoted: false,
            });
            continue;
        }

        if ch == '\'' || ch == '"' {
            let quote = ch;
            let mut value = String::new();
            let mut escaped = false;

            let mut terminated = false;
            for c in chars.by_ref() {
                if escaped {
                    value.push(c);
                    escaped = false;
                    continue;
                }
                if c == '\\' {
                    escaped = true;
                    continue;
                }
                if c == quote {
                    terminated = true;
                    break;
                }
                value.push(c);
            }

            if !terminated {
                return Err("Unterminated quoted string in SQL schema definition".to_string());
            }

            tokens.push(SqlToken {
                text: value,
                quoted: true,
            });
            continue;
        }

        let mut text = String::new();
        text.push(ch);

        while let Some(next) = chars.peek().copied() {
            if next.is_whitespace() || matches!(next, '(' | ')' | ',' | ';' | '=') {
                break;
            }
            if next == '#' {
                break;
            }
            if next == '-' {
                let mut clone = chars.clone();
                let _ = clone.next();
                if let Some('-') = clone.next() {
                    break;
                }
            }
            text.push(next);
            let _ = chars.next();
        }

        tokens.push(SqlToken {
            text,
            quoted: false,
        });
    }

    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_compact_json_schema() {
        let input = serde_json::json!({
            "name": "Person",
            "properties": {
                "email": { "type": "string", "required": true, "index": "unique" },
                "age": { "type": "int", "index": "range" }
            },
            "meta": {
                "allow_undeclared_edges": true,
                "extras_policy": "warn"
            }
        })
        .to_string();

        let schema =
            parse_schema_input(&input, SchemaInputFormat::Auto).expect("schema should parse");
        assert_eq!(schema.name, "Person");
        assert_eq!(schema.properties.len(), 2);
        assert_eq!(
            schema.properties["email"].index,
            Some(IndexType::Unique as i32)
        );
        assert_eq!(
            schema.meta.expect("meta should exist").extras_policy,
            ExtrasPolicy::Warn as i32
        );
    }

    #[test]
    fn parses_protobuf_style_json_schema() {
        let input = serde_json::json!({
            "name": "Person",
            "properties": {
                "active": {
                    "type": "PROPERTY_TYPE_BOOL",
                    "default_value": { "bool_value": true }
                }
            },
            "edges": {
                "WORKS_AT": {
                    "target": { "specificType": "Company" },
                    "direction": "EDGE_DIRECTION_DEF_OUTGOING",
                    "ownership": "OWNERSHIP_MODE_SOURCE_SITE"
                }
            },
            "meta": {
                "allowUndeclaredEdges": false,
                "extrasPolicy": "EXTRAS_POLICY_REJECT"
            }
        })
        .to_string();

        let schema =
            parse_schema_input(&input, SchemaInputFormat::Json).expect("schema should parse");
        assert_eq!(schema.name, "Person");
        assert!(schema.edges.contains_key("WORKS_AT"));
        let active = &schema.properties["active"];
        assert_eq!(active.r#type, PropertyType::Bool as i32);
        assert!(matches!(
            active.default_value.as_ref().and_then(|v| v.kind.as_ref()),
            Some(value::Kind::BoolValue(true))
        ));
    }

    #[test]
    fn parses_sql_schema_with_edges_and_with_clause() {
        let input = r#"
            CREATE TYPE Person (
                email STRING REQUIRED INDEX UNIQUE,
                age INT INDEX RANGE,
                active BOOL DEFAULT true
            )
            WITH (allow_undeclared_edges = false, extras_policy = reject)
            EDGE WORKS_AT TO Company (
                since TIMESTAMP,
                role STRING
            ) DIRECTION OUTGOING OWNERSHIP SOURCE_SITE SORT_KEY since;
        "#;

        let schema =
            parse_schema_input(input, SchemaInputFormat::Sql).expect("schema should parse");
        assert_eq!(schema.name, "Person");
        assert_eq!(schema.properties["email"].required, true);
        assert_eq!(
            schema.properties["age"].index,
            Some(IndexType::Range as i32)
        );

        let edge = schema.edges.get("WORKS_AT").expect("edge should exist");
        assert_eq!(edge.direction, EdgeDirectionDef::Outgoing as i32);
        assert_eq!(edge.sort_key, "since");
        assert!(edge.properties.contains_key("since"));

        let meta = schema.meta.expect("meta should exist");
        assert!(!meta.allow_undeclared_edges);
        assert_eq!(meta.extras_policy, ExtrasPolicy::Reject as i32);
    }

    #[test]
    fn auto_mode_falls_back_to_sql() {
        let input = "CREATE SCHEMA Team (name STRING REQUIRED);";
        let schema = parse_schema_input(input, SchemaInputFormat::Auto).expect("sql should parse");
        assert_eq!(schema.name, "Team");
        assert!(schema.properties.contains_key("name"));
    }
}
