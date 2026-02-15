use pelago_proto::pelago::v1::{
    admin_service_client::AdminServiceClient,
    edge_service_client::EdgeServiceClient,
    node_service_client::NodeServiceClient,
    query_service_client::QueryServiceClient,
    schema_service_client::SchemaServiceClient,
};
use tonic::transport::Channel;

pub struct GrpcConnection {
    channel: Channel,
}

impl GrpcConnection {
    pub async fn connect(server: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let channel = Channel::from_shared(server.to_string())?
            .connect()
            .await?;
        Ok(Self { channel })
    }

    pub fn schema_client(&self) -> SchemaServiceClient<Channel> {
        SchemaServiceClient::new(self.channel.clone())
    }

    pub fn node_client(&self) -> NodeServiceClient<Channel> {
        NodeServiceClient::new(self.channel.clone())
    }

    pub fn edge_client(&self) -> EdgeServiceClient<Channel> {
        EdgeServiceClient::new(self.channel.clone())
    }

    pub fn query_client(&self) -> QueryServiceClient<Channel> {
        QueryServiceClient::new(self.channel.clone())
    }

    pub fn admin_client(&self) -> AdminServiceClient<Channel> {
        AdminServiceClient::new(self.channel.clone())
    }

    pub fn channel(&self) -> &Channel {
        &self.channel
    }
}
