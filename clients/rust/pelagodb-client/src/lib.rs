use anyhow::Result;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;

pub mod pelago {
    tonic::include_proto!("pelago.v1");
}

use pelago::{
    node_service_client::NodeServiceClient, query_service_client::QueryServiceClient, CreateNodeRequest,
    FindNodesRequest, GetNodeRequest, Node, ReadConsistency, RequestContext, Value,
};

pub struct PelagoClient {
    database: String,
    namespace: String,
    site_id: String,
    bearer_token: Option<String>,
    api_key: Option<String>,
    node: NodeServiceClient<Channel>,
    query: QueryServiceClient<Channel>,
}

impl PelagoClient {
    pub async fn connect(endpoint: &str, database: &str, namespace: &str) -> Result<Self> {
        let node = NodeServiceClient::connect(endpoint.to_string()).await?;
        let query = QueryServiceClient::connect(endpoint.to_string()).await?;
        Ok(Self {
            database: database.to_string(),
            namespace: namespace.to_string(),
            site_id: String::new(),
            bearer_token: None,
            api_key: None,
            node,
            query,
        })
    }

    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    pub fn with_bearer_token(mut self, bearer_token: impl Into<String>) -> Self {
        self.bearer_token = Some(bearer_token.into());
        self
    }

    fn context(&self) -> RequestContext {
        RequestContext {
            database: self.database.clone(),
            namespace: self.namespace.clone(),
            site_id: self.site_id.clone(),
            request_id: uuid_like(),
        }
    }

    fn apply_auth_metadata<T>(&self, req: &mut tonic::Request<T>) {
        if let Some(api_key) = &self.api_key {
            if let Ok(v) = MetadataValue::try_from(api_key.as_str()) {
                req.metadata_mut().insert("x-api-key", v);
            }
        }
        if let Some(token) = &self.bearer_token {
            let value = format!("Bearer {}", token);
            if let Ok(v) = MetadataValue::try_from(value.as_str()) {
                req.metadata_mut().insert("authorization", v);
            }
        }
    }

    pub async fn create_node<I>(&mut self, entity_type: &str, properties: I) -> Result<Node>
    where
        I: IntoIterator<Item = (&'static str, Value)>,
    {
        let mut req = tonic::Request::new(CreateNodeRequest {
            context: Some(self.context()),
            entity_type: entity_type.to_string(),
            properties: properties
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        });
        self.apply_auth_metadata(&mut req);
        let resp = self.node.create_node(req).await?.into_inner();
        resp.node.ok_or_else(|| anyhow::anyhow!("CreateNode returned no node"))
    }

    pub async fn get_node(&mut self, entity_type: &str, node_id: &str) -> Result<Option<Node>> {
        let mut req = tonic::Request::new(GetNodeRequest {
            context: Some(self.context()),
            entity_type: entity_type.to_string(),
            node_id: node_id.to_string(),
            consistency: ReadConsistency::Strong as i32,
            fields: Vec::new(),
        });
        self.apply_auth_metadata(&mut req);
        Ok(self.node.get_node(req).await?.into_inner().node)
    }

    pub async fn find_nodes(
        &mut self,
        entity_type: &str,
        cel_expression: &str,
        limit: u32,
    ) -> Result<Vec<Node>> {
        let mut req = tonic::Request::new(FindNodesRequest {
            context: Some(self.context()),
            entity_type: entity_type.to_string(),
            cel_expression: cel_expression.to_string(),
            consistency: ReadConsistency::Strong as i32,
            fields: Vec::new(),
            limit,
            cursor: Vec::new(),
        });
        self.apply_auth_metadata(&mut req);
        let mut stream = self.query.find_nodes(req).await?.into_inner();
        let mut out = Vec::new();
        while let Some(item) = stream.message().await? {
            if let Some(node) = item.node {
                out.push(node);
            }
        }
        Ok(out)
    }
}

pub fn value_string(v: impl Into<String>) -> Value {
    Value {
        kind: Some(pelago::value::Kind::StringValue(v.into())),
    }
}

pub fn value_int(v: i64) -> Value {
    Value {
        kind: Some(pelago::value::Kind::IntValue(v)),
    }
}

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros();
    format!("req-{}", ts)
}
