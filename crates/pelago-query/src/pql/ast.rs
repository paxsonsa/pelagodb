use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PqlQuery {
    pub name: Option<String>,
    pub default_namespace: Option<String>,
    pub blocks: Vec<QueryBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryBlock {
    pub name: String,
    pub root: RootFunction,
    pub directives: Vec<Directive>,
    pub selections: Vec<Selection>,
    pub capture_as: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RootFunction {
    Uid(QualifiedRef),
    UidVar(String),
    UidSet(Vec<String>, SetOp),
    Type(QualifiedType),
    Eq(String, LiteralValue),
    Ge(String, LiteralValue),
    Le(String, LiteralValue),
    Gt(String, LiteralValue),
    Lt(String, LiteralValue),
    Between(String, LiteralValue, LiteralValue),
    Has(String),
    AllOfTerms(String, String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualifiedRef {
    pub namespace: Option<String>,
    pub entity_type: String,
    pub node_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualifiedType {
    pub namespace: Option<String>,
    pub entity_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SetOp {
    Union,
    Intersect,
    Difference,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LiteralValue {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Selection {
    Field(String),
    Edge(EdgeTraversal),
    Aggregate(AggregateExpr),
    ValueVar(String, AggregateExpr),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeTraversal {
    pub edge_namespace: Option<String>,
    pub edge_type: String,
    pub direction: PqlEdgeDirection,
    pub target_type: Option<TargetSpec>,
    pub directives: Vec<Directive>,
    pub selections: Vec<Selection>,
    pub capture_as: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetSpec {
    pub namespace: Option<String>,
    pub entity_type: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PqlEdgeDirection {
    Outgoing,
    Incoming,
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Directive {
    Filter(String),
    Edge(String),
    Cascade,
    Limit {
        first: u32,
        offset: Option<u32>,
    },
    Sort {
        field: String,
        desc: bool,
        on_edge: bool,
    },
    Facets(Vec<String>),
    Recurse {
        depth: u32,
    },
    GroupBy(Vec<String>),
    Explain,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AggregateExpr {
    Count(Box<Selection>),
    Sum(String),
    Avg(String),
    Min(String),
    Max(String),
}

impl std::fmt::Display for PqlEdgeDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PqlEdgeDirection::Outgoing => write!(f, "->"),
            PqlEdgeDirection::Incoming => write!(f, "<-"),
            PqlEdgeDirection::Both => write!(f, "<->"),
        }
    }
}
