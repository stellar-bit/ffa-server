

use stellar_bit_central_hub_api::HubAPI;
use stellar_bit_server_template::{start_server, ServerHubConn, SERVER_NAME};


#[tokio::main]
async fn main() {
    color_backtrace::install();

    // ask for username, password
    let username: String = dialoguer::Input::new()
        .with_prompt("Username")
        .interact_text()
        .unwrap();

    let password: String = dialoguer::Password::new()
        .with_prompt("Password")
        .interact()
        .unwrap();

    let api = HubAPI::connect(username.clone(), password).await.unwrap();

    // let user select which server he wants to use from his servers, give link to page where he can create new one
    // once he does that establish server hub connection

    let servers = api.servers().await;

    let user_data = api.user_data_username(&username).await;
    let server_details = servers.into_iter().find(|x| x.name == SERVER_NAME && x.owner_id == user_data.id).expect(&format!("You don't own a server with the name {:?}.\nMaybe you've meant to create it first? You can do so on the official Stellar Bit website.", SERVER_NAME));

    let hub_conn = ServerHubConn::new(api, server_details.id).await;

    start_server(hub_conn).await;
}
