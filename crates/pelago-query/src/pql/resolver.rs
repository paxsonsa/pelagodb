use super::ast::*;
use std::collections::HashMap;

/// Errors during PQL resolution/compilation
#[derive(Debug, thiserror::Error)]
pub enum PqlError {
    #[error("Unknown entity type: {0}")]
    UnknownEntityType(String),
    #[error("Unknown field '{field}' on type '{entity_type}'")]
    UnknownField { entity_type: String, field: String },
    #[error("Unknown edge type '{edge_type}' on type '{entity_type}'")]
    UnknownEdge {
        entity_type: String,
        edge_type: String,
    },
    #[error("Undefined variable: ${0}")]
    UndefinedVariable(String),
    #[error("Circular variable dependency involving: {0}")]
    CircularDependency(String),
    #[error("Invalid root function: {0}")]
    InvalidRootFunction(String),
    #[error("Compilation error: {0}")]
    CompilationError(String),
}

/// Schema information needed for resolution (kept simple — no FDB dependency)
pub struct SchemaInfo {
    pub entity_type: String,
    pub fields: Vec<String>,
    pub edges: Vec<String>,
    pub allow_undeclared_edges: bool,
}

/// A simple schema provider trait for resolution
pub trait SchemaProvider {
    fn get_schema(&self, entity_type: &str) -> Option<&SchemaInfo>;
}

/// In-memory schema provider for testing
pub struct InMemorySchemaProvider {
    schemas: HashMap<String, SchemaInfo>,
}

impl InMemorySchemaProvider {
    pub fn new() -> Self {
        Self {
            schemas: HashMap::new(),
        }
    }

    pub fn add_schema(&mut self, schema: SchemaInfo) {
        self.schemas.insert(schema.entity_type.clone(), schema);
    }
}

impl Default for InMemorySchemaProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl SchemaProvider for InMemorySchemaProvider {
    fn get_schema(&self, entity_type: &str) -> Option<&SchemaInfo> {
        self.schemas.get(entity_type)
    }
}

/// Resolved query with execution ordering
pub struct ResolvedQuery {
    pub blocks: Vec<ResolvedBlock>,
    pub execution_order: Vec<usize>,
    pub variables: HashMap<String, VariableInfo>,
}

pub struct ResolvedBlock {
    pub name: String,
    pub block: QueryBlock,
    pub entity_type: String,
    pub depends_on: Vec<String>,
}

pub struct VariableInfo {
    pub defined_in_block: usize,
    pub entity_type: String,
}

pub struct PqlResolver;

impl PqlResolver {
    pub fn new() -> Self {
        Self
    }

    /// Resolve a PQL query against schemas
    pub fn resolve<S: SchemaProvider>(
        &self,
        query: &PqlQuery,
        schemas: &S,
    ) -> Result<ResolvedQuery, PqlError> {
        let mut resolved_blocks = Vec::new();
        let mut variables: HashMap<String, VariableInfo> = HashMap::new();

        // First pass: resolve entity types and register variables
        for (i, block) in query.blocks.iter().enumerate() {
            let (entity_type, depends_on) =
                self.resolve_root_function(&block.root, &variables)?;

            // Validate entity type exists in schema
            let schema = schemas
                .get_schema(&entity_type)
                .ok_or_else(|| PqlError::UnknownEntityType(entity_type.clone()))?;

            // Validate fields in selections
            self.validate_selections(&block.selections, schema)?;

            // Register variable if block captures one
            if let Some(ref var_name) = block.capture_as {
                variables.insert(
                    var_name.clone(),
                    VariableInfo {
                        defined_in_block: i,
                        entity_type: entity_type.clone(),
                    },
                );
            }

            resolved_blocks.push(ResolvedBlock {
                name: block.name.clone(),
                block: block.clone(),
                entity_type,
                depends_on,
            });
        }

        // Build dependency graph and topological sort
        let execution_order = self.topological_sort(&resolved_blocks, &variables)?;

        Ok(ResolvedQuery {
            blocks: resolved_blocks,
            execution_order,
            variables,
        })
    }

    /// Resolve a root function to its entity type and variable dependencies
    fn resolve_root_function(
        &self,
        root: &RootFunction,
        variables: &HashMap<String, VariableInfo>,
    ) -> Result<(String, Vec<String>), PqlError> {
        match root {
            RootFunction::Type(qt) => Ok((qt.entity_type.clone(), vec![])),
            RootFunction::Uid(qref) => Ok((qref.entity_type.clone(), vec![])),
            RootFunction::UidVar(var) => {
                let info = variables
                    .get(var)
                    .ok_or_else(|| PqlError::UndefinedVariable(var.clone()))?;
                Ok((info.entity_type.clone(), vec![var.clone()]))
            }
            RootFunction::UidSet(vars, _) => {
                // All variables must be defined; use the first variable's type
                let mut deps = Vec::new();
                let mut entity_type = None;
                for var in vars {
                    let info = variables
                        .get(var)
                        .ok_or_else(|| PqlError::UndefinedVariable(var.clone()))?;
                    if entity_type.is_none() {
                        entity_type = Some(info.entity_type.clone());
                    }
                    deps.push(var.clone());
                }
                Ok((
                    entity_type.unwrap_or_default(),
                    deps,
                ))
            }
            RootFunction::Eq(_, _)
            | RootFunction::Ge(_, _)
            | RootFunction::Le(_, _)
            | RootFunction::Gt(_, _)
            | RootFunction::Lt(_, _)
            | RootFunction::Between(_, _, _)
            | RootFunction::Has(_)
            | RootFunction::AllOfTerms(_, _) => {
                // For filter-based root functions in full-form queries, the entity type
                // must come from context. We use the block name as a convention — the
                // block name should match or the schema provider should resolve it.
                // In practice, these are used with type() in short form or the block
                // name maps to a type. We return the function name as a placeholder
                // that the caller can override. For now, we return a compilation error
                // since these need a type context.
                Err(PqlError::CompilationError(
                    "Filter root functions (eq, ge, le, etc.) require a type() wrapper or short-form syntax".to_string(),
                ))
            }
        }
    }

    /// Validate selections against schema
    fn validate_selections(
        &self,
        selections: &[Selection],
        schema: &SchemaInfo,
    ) -> Result<(), PqlError> {
        for sel in selections {
            match sel {
                Selection::Field(name) => {
                    if !schema.fields.contains(name) {
                        return Err(PqlError::UnknownField {
                            entity_type: schema.entity_type.clone(),
                            field: name.clone(),
                        });
                    }
                }
                Selection::Edge(edge) => {
                    if !schema.allow_undeclared_edges
                        && !schema.edges.contains(&edge.edge_type)
                    {
                        return Err(PqlError::UnknownEdge {
                            entity_type: schema.entity_type.clone(),
                            edge_type: edge.edge_type.clone(),
                        });
                    }
                }
                Selection::Aggregate(_) | Selection::ValueVar(_, _) => {
                    // Aggregates are validated at execution time
                }
            }
        }
        Ok(())
    }

    /// Topological sort of blocks by variable dependencies
    fn topological_sort(
        &self,
        blocks: &[ResolvedBlock],
        variables: &HashMap<String, VariableInfo>,
    ) -> Result<Vec<usize>, PqlError> {
        let n = blocks.len();

        // Build adjacency: block i depends on block j if i uses a variable defined in j
        let mut in_degree = vec![0usize; n];
        let mut adj: Vec<Vec<usize>> = vec![vec![]; n];

        for (i, block) in blocks.iter().enumerate() {
            for dep_var in &block.depends_on {
                if let Some(info) = variables.get(dep_var) {
                    let j = info.defined_in_block;
                    adj[j].push(i);
                    in_degree[i] += 1;
                }
            }
        }

        // Kahn's algorithm
        let mut queue: Vec<usize> = (0..n).filter(|&i| in_degree[i] == 0).collect();
        let mut order = Vec::with_capacity(n);

        while let Some(node) = queue.pop() {
            order.push(node);
            for &next in &adj[node] {
                in_degree[next] -= 1;
                if in_degree[next] == 0 {
                    queue.push(next);
                }
            }
        }

        if order.len() != n {
            // Find nodes still with nonzero in_degree (part of a cycle)
            let cycle_blocks: Vec<String> = (0..n)
                .filter(|&i| in_degree[i] > 0)
                .map(|i| blocks[i].name.clone())
                .collect();
            return Err(PqlError::CircularDependency(cycle_blocks.join(", ")));
        }

        Ok(order)
    }
}

impl Default for PqlResolver {
    fn default() -> Self {
        Self::new()
    }
}
