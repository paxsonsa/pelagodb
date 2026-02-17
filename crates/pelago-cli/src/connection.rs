use pelago_proto::pelago::v1::{
    admin_service_client::AdminServiceClient, edge_service_client::EdgeServiceClient,
    node_service_client::NodeServiceClient, query_service_client::QueryServiceClient,
    schema_service_client::SchemaServiceClient,
};
use std::sync::OnceLock;
use tonic::metadata::MetadataValue;
use tonic::service::interceptor::InterceptedService;
use tonic::service::Interceptor;
use tonic::transport::Channel;

#[derive(Clone, Debug, Default)]
pub struct AuthConfig {
    pub api_key: Option<String>,
    pub bearer_token: Option<String>,
}

static AUTH_CONFIG: OnceLock<AuthConfig> = OnceLock::new();

#[derive(Clone, Debug, Default)]
pub(crate) struct AuthInterceptor {
    api_key: Option<String>,
    bearer_token: Option<String>,
}

impl Interceptor for AuthInterceptor {
    fn call(
        &mut self,
        mut request: tonic::Request<()>,
    ) -> Result<tonic::Request<()>, tonic::Status> {
        if let Some(api_key) = &self.api_key {
            let value = MetadataValue::try_from(api_key.as_str())
                .map_err(|_| tonic::Status::unauthenticated("invalid API key value"))?;
            request.metadata_mut().insert("x-api-key", value);
        }

        if let Some(bearer) = &self.bearer_token {
            let auth_value = format!("Bearer {}", bearer);
            let value = MetadataValue::try_from(auth_value.as_str())
                .map_err(|_| tonic::Status::unauthenticated("invalid bearer token value"))?;
            request.metadata_mut().insert("authorization", value);
        }

        Ok(request)
    }
}

pub struct GrpcConnection {
    channel: Channel,
    interceptor: AuthInterceptor,
}

pub fn set_auth_config(config: AuthConfig) {
    let _ = AUTH_CONFIG.set(config);
}

fn current_auth_interceptor() -> AuthInterceptor {
    let config = AUTH_CONFIG.get().cloned().unwrap_or_default();
    AuthInterceptor {
        api_key: config.api_key,
        bearer_token: config.bearer_token,
    }
}

impl GrpcConnection {
    pub async fn connect(server: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let channel = Channel::from_shared(server.to_string())?.connect().await?;
        Ok(Self {
            channel,
            interceptor: current_auth_interceptor(),
        })
    }

    pub fn schema_client(
        &self,
    ) -> SchemaServiceClient<InterceptedService<Channel, AuthInterceptor>> {
        SchemaServiceClient::with_interceptor(self.channel.clone(), self.interceptor.clone())
    }

    pub fn node_client(&self) -> NodeServiceClient<InterceptedService<Channel, AuthInterceptor>> {
        NodeServiceClient::with_interceptor(self.channel.clone(), self.interceptor.clone())
    }

    pub fn edge_client(&self) -> EdgeServiceClient<InterceptedService<Channel, AuthInterceptor>> {
        EdgeServiceClient::with_interceptor(self.channel.clone(), self.interceptor.clone())
    }

    pub fn query_client(&self) -> QueryServiceClient<InterceptedService<Channel, AuthInterceptor>> {
        QueryServiceClient::with_interceptor(self.channel.clone(), self.interceptor.clone())
    }

    pub fn admin_client(&self) -> AdminServiceClient<InterceptedService<Channel, AuthInterceptor>> {
        AdminServiceClient::with_interceptor(self.channel.clone(), self.interceptor.clone())
    }
}
