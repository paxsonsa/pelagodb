use super::ast::*;
use super::resolver::{PqlError, ResolvedQuery};

/// Compiled block — ready for execution against gRPC or storage layer
#[derive(Debug, Clone)]
pub enum CompiledBlock {
    /// Single node lookup by ID
    PointLookup {
        block_name: String,
        entity_type: String,
        node_id: String,
        fields: Vec<String>,
    },
    /// FindNodes query with optional filter
    FindNodes {
        block_name: String,
        entity_type: String,
        cel_expression: Option<String>,
        fields: Vec<String>,
        limit: Option<u32>,
        offset: Option<u32>,
    },
    /// Multi-hop traversal
    Traverse {
        block_name: String,
        start_entity_type: String,
        start_node_id: String,
        steps: Vec<CompiledStep>,
        max_depth: u32,
        cascade: bool,
        max_results: u32,
    },
    /// Variable reference (reads from captured results)
    VariableRef {
        block_name: String,
        variable: String,
        filter: Option<String>,
        fields: Vec<String>,
        limit: Option<u32>,
        offset: Option<u32>,
    },
    /// Variable set operation (union/intersect/difference)
    VariableSet {
        block_name: String,
        variables: Vec<String>,
        set_op: SetOp,
        filter: Option<String>,
        fields: Vec<String>,
        limit: Option<u32>,
        offset: Option<u32>,
    },
}

#[derive(Debug, Clone)]
pub struct CompiledStep {
    pub edge_type: String,
    pub direction: PqlEdgeDirection,
    pub edge_filter: Option<String>,
    pub node_filter: Option<String>,
    pub fields: Vec<String>,
    pub edge_fields: Vec<String>,
    pub per_node_limit: Option<u32>,
    pub sort: Option<CompiledSort>,
}

#[derive(Debug, Clone)]
pub struct CompiledSort {
    pub field: String,
    pub descending: bool,
    pub on_edge: bool,
}

pub struct PqlCompiler;

impl PqlCompiler {
    pub fn new() -> Self {
        Self
    }

    /// Compile a resolved query into executable blocks
    pub fn compile(&self, resolved: &ResolvedQuery) -> Result<Vec<CompiledBlock>, PqlError> {
        let mut compiled = Vec::new();
        for &idx in &resolved.execution_order {
            let rb = &resolved.blocks[idx];
            let block = self.compile_block(&rb.block, &rb.entity_type)?;
            compiled.push(block);
        }
        Ok(compiled)
    }

    /// Compile a single block
    fn compile_block(
        &self,
        block: &QueryBlock,
        entity_type: &str,
    ) -> Result<CompiledBlock, PqlError> {
        self.validate_block_directives(block, has_edge_traversals(&block.selections))?;

        match &block.root {
            RootFunction::Uid(qref) => {
                if has_edge_traversals(&block.selections) {
                    let steps = compile_edge_traversals(&block.selections)?;
                    let max_depth = self.extract_recurse_depth(&block.directives);
                    let cascade = block
                        .directives
                        .iter()
                        .any(|d| matches!(d, Directive::Cascade));
                    let max_results = self.extract_limit(&block.directives).unwrap_or(1000);
                    Ok(CompiledBlock::Traverse {
                        block_name: block.name.clone(),
                        start_entity_type: entity_type.to_string(),
                        start_node_id: qref.node_id.clone(),
                        steps,
                        max_depth,
                        cascade,
                        max_results,
                    })
                } else {
                    let fields = extract_fields(&block.selections);
                    Ok(CompiledBlock::PointLookup {
                        block_name: block.name.clone(),
                        entity_type: entity_type.to_string(),
                        node_id: qref.node_id.clone(),
                        fields,
                    })
                }
            }
            RootFunction::Type(_qt) => {
                let fields = extract_fields(&block.selections);
                let cel = self.extract_filter_cel(&block.directives);
                let limit = self.extract_limit(&block.directives);
                let offset = self.extract_offset(&block.directives);
                Ok(CompiledBlock::FindNodes {
                    block_name: block.name.clone(),
                    entity_type: entity_type.to_string(),
                    cel_expression: cel,
                    fields,
                    limit,
                    offset,
                })
            }
            RootFunction::UidVar(var) => {
                let fields = extract_fields(&block.selections);
                let filter = self.extract_filter_cel(&block.directives);
                let limit = self.extract_limit(&block.directives);
                let offset = self.extract_offset(&block.directives);
                Ok(CompiledBlock::VariableRef {
                    block_name: block.name.clone(),
                    variable: var.clone(),
                    filter,
                    fields,
                    limit,
                    offset,
                })
            }
            RootFunction::Eq(_, _)
            | RootFunction::Ge(_, _)
            | RootFunction::Le(_, _)
            | RootFunction::Gt(_, _)
            | RootFunction::Lt(_, _)
            | RootFunction::Between(_, _, _)
            | RootFunction::Has(_)
            | RootFunction::AllOfTerms(_, _) => {
                // These are filter-based roots from short-form queries
                let fields = extract_fields(&block.selections);
                let cel = root_to_cel(&block.root);
                let limit = self.extract_limit(&block.directives);
                let offset = self.extract_offset(&block.directives);
                Ok(CompiledBlock::FindNodes {
                    block_name: block.name.clone(),
                    entity_type: entity_type.to_string(),
                    cel_expression: cel,
                    fields,
                    limit,
                    offset,
                })
            }
            RootFunction::UidSet(vars, set_op) => {
                let fields = extract_fields(&block.selections);
                let filter = self.extract_filter_cel(&block.directives);
                let limit = self.extract_limit(&block.directives);
                let offset = self.extract_offset(&block.directives);
                Ok(CompiledBlock::VariableSet {
                    block_name: block.name.clone(),
                    variables: vars.clone(),
                    set_op: set_op.clone(),
                    filter,
                    fields,
                    limit,
                    offset,
                })
            }
        }
    }

    fn validate_block_directives(
        &self,
        block: &QueryBlock,
        has_edges: bool,
    ) -> Result<(), PqlError> {
        for directive in &block.directives {
            let supported = match &block.root {
                RootFunction::Uid(_) if has_edges => matches!(
                    directive,
                    Directive::Limit { .. } | Directive::Cascade | Directive::Recurse { .. }
                ),
                RootFunction::Type(_)
                | RootFunction::UidVar(_)
                | RootFunction::UidSet(_, _)
                | RootFunction::Eq(_, _)
                | RootFunction::Ge(_, _)
                | RootFunction::Le(_, _)
                | RootFunction::Gt(_, _)
                | RootFunction::Lt(_, _)
                | RootFunction::Between(_, _, _)
                | RootFunction::Has(_)
                | RootFunction::AllOfTerms(_, _) => {
                    matches!(directive, Directive::Filter(_) | Directive::Limit { .. })
                }
                RootFunction::Uid(_) => false,
            };

            if !supported {
                return Err(PqlError::CompilationError(format!(
                    "unsupported block directive {} on block '{}'",
                    directive_name(directive),
                    block.name
                )));
            }
        }
        Ok(())
    }

    /// Extract filter CEL expression from directives
    fn extract_filter_cel(&self, directives: &[Directive]) -> Option<String> {
        for d in directives {
            if let Directive::Filter(expr) = d {
                return Some(expr.clone());
            }
        }
        None
    }

    /// Extract limit from directives
    fn extract_limit(&self, directives: &[Directive]) -> Option<u32> {
        for d in directives {
            if let Directive::Limit { first, .. } = d {
                return Some(*first);
            }
        }
        None
    }

    /// Extract offset from directives
    fn extract_offset(&self, directives: &[Directive]) -> Option<u32> {
        for d in directives {
            if let Directive::Limit { offset, .. } = d {
                return *offset;
            }
        }
        None
    }

    /// Extract recurse depth from directives, defaulting to number of edge steps
    fn extract_recurse_depth(&self, directives: &[Directive]) -> u32 {
        for d in directives {
            if let Directive::Recurse { depth } = d {
                return *depth;
            }
        }
        1
    }
}

impl Default for PqlCompiler {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract field names from selections
pub fn extract_fields(selections: &[Selection]) -> Vec<String> {
    selections
        .iter()
        .filter_map(|s| match s {
            Selection::Field(name) => Some(name.clone()),
            _ => None,
        })
        .collect()
}

/// Check if block has edge traversals
pub fn has_edge_traversals(selections: &[Selection]) -> bool {
    selections.iter().any(|s| matches!(s, Selection::Edge(_)))
}

/// Compile edge traversals to steps
pub fn compile_edge_traversals(selections: &[Selection]) -> Result<Vec<CompiledStep>, PqlError> {
    selections
        .iter()
        .filter_map(|s| {
            if let Selection::Edge(edge) = s {
                Some(compile_single_edge(edge))
            } else {
                None
            }
        })
        .collect()
}

fn compile_single_edge(edge: &EdgeTraversal) -> Result<CompiledStep, PqlError> {
    let mut edge_filter = None;
    let mut node_filter = None;
    let mut per_node_limit = None;
    let mut sort = None;
    let mut edge_fields = Vec::new();

    for d in &edge.directives {
        match d {
            Directive::Edge(expr) => edge_filter = Some(expr.clone()),
            Directive::Filter(expr) => node_filter = Some(expr.clone()),
            Directive::Limit { first, .. } => per_node_limit = Some(*first),
            Directive::Sort {
                field,
                desc,
                on_edge,
            } => {
                sort = Some(CompiledSort {
                    field: field.clone(),
                    descending: *desc,
                    on_edge: *on_edge,
                });
            }
            Directive::Facets(fields) => {
                edge_fields.extend(fields.iter().cloned());
            }
            other => {
                return Err(PqlError::CompilationError(format!(
                    "unsupported edge directive {} on edge '{}'",
                    directive_name(other),
                    edge.edge_type
                )));
            }
        }
    }

    let fields = extract_fields(&edge.selections);

    Ok(CompiledStep {
        edge_type: edge.edge_type.clone(),
        direction: edge.direction,
        edge_filter,
        node_filter,
        fields,
        edge_fields,
        per_node_limit,
        sort,
    })
}

fn directive_name(directive: &Directive) -> &'static str {
    match directive {
        Directive::Filter(_) => "@filter",
        Directive::Edge(_) => "@edge",
        Directive::Cascade => "@cascade",
        Directive::Limit { .. } => "@limit",
        Directive::Sort { .. } => "@sort",
        Directive::Facets(_) => "@facets",
        Directive::Recurse { .. } => "@recurse",
        Directive::GroupBy(_) => "@groupby",
        Directive::Explain => "@explain",
    }
}

/// Build CEL expression from root function
pub fn root_to_cel(root: &RootFunction) -> Option<String> {
    match root {
        RootFunction::Eq(field, val) => Some(format!("{} == {}", field, literal_to_cel(val))),
        RootFunction::Ge(field, val) => Some(format!("{} >= {}", field, literal_to_cel(val))),
        RootFunction::Le(field, val) => Some(format!("{} <= {}", field, literal_to_cel(val))),
        RootFunction::Gt(field, val) => Some(format!("{} > {}", field, literal_to_cel(val))),
        RootFunction::Lt(field, val) => Some(format!("{} < {}", field, literal_to_cel(val))),
        RootFunction::Between(field, lo, hi) => Some(format!(
            "{} >= {} && {} <= {}",
            field,
            literal_to_cel(lo),
            field,
            literal_to_cel(hi)
        )),
        RootFunction::Has(field) => Some(format!("has({})", field)),
        RootFunction::AllOfTerms(field, terms) => {
            Some(format!("allofterms({}, \"{}\")", field, terms))
        }
        _ => None,
    }
}

fn literal_to_cel(val: &LiteralValue) -> String {
    match val {
        LiteralValue::String(s) => format!("\"{}\"", s),
        LiteralValue::Int(n) => n.to_string(),
        LiteralValue::Float(f) => f.to_string(),
        LiteralValue::Bool(b) => b.to_string(),
    }
}
