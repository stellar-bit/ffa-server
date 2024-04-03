use stellar_bit_central_hub_api::HubAPI;
use reqwest::{Client, ClientBuilder};

use crate::SERVER_ADDR;

/// # Hub Connection
/// **Purpose**
/// - Verifies authority of users when accessing server
/// - Lists this server along with its server address publicly
pub struct ServerHubConn {
    server_id: i64,
    api: HubAPI
}

impl ServerHubConn {
    pub async fn new(api: HubAPI, server_id: i64) -> Self {
        Self {
            api,
            server_id
        }
    }
    pub async fn keep_alive(&self) {
        self.api.server_keep_alive(self.server_id, &SERVER_ADDR.to_string()).await;
    }
    pub async fn verify(&self, user_id: i64, acc_token: String) -> bool {
        self.api.verify_token(self.server_id, user_id, acc_token).await
    }
}