use futures::prelude::sink::SinkExt;
use tokio::task::yield_now;
use tokio::time::{interval, Instant};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use tokio::sync::{RwLock, Mutex};
use std::time::Duration;
use stellar_bit_core::prelude::*;
use stellar_bit_core::{
    game::GameCmdExecutionError,
    network::{ClientRequest, ServerResponse},
};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{accept_async, tungstenite::Message};

mod client_handle;
use client_handle::ClientHandle;

mod hub_connection;
pub use hub_connection::ServerHubConn;


pub const SERVER_ADDR: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(213, 211, 62, 122), 39453);
pub const SERVER_NAME: &str = "Free for all";

pub async fn start_server(hub_conn: ServerHubConn) {
    let server_state = Arc::new(RwLock::new(ServerState::default()));
    let server_session = ServerSession::new(server_state.clone());

    let game_clone = server_session.game.clone();
    tokio::task::spawn(handle_connections(game_clone, hub_conn, server_state));

    server_session.start(60).await.unwrap();
}

#[derive(Default, Clone, Copy, PartialEq)]
enum ServerState {
    #[default]
    Lobby,
    InitGame,
    Game,
}

struct ServerSession {
    game: Arc<RwLock<Game>>,
    state: Arc<RwLock<ServerState>>,
}

impl ServerSession {
    pub fn new(state: Arc<RwLock<ServerState>>) -> Self {
        Self {
            game: Arc::new(RwLock::new(Game::new())),
            state
        }
    }
    pub async fn start(mut self, fps: u32) -> Result<(), GameCmdExecutionError> {
        let target_duration = std::time::Duration::from_secs_f32(1. / fps as f32);
        
        let mut lobby_timer: Option<Instant> = None;
        const LOBBY_TIME: Duration = Duration::from_secs(60);
        let mut lobby_report_interval = Interval::new(Duration::from_secs(1));

        let mut game_timer: Option<Instant> = None;
        const GAME_TIME: Duration = Duration::from_secs(60*10);

        loop {
            let frame_time_measure = std::time::Instant::now();

            let mut game = self.game.write().await;

            let cur_state = *self.state.read().await;
            match cur_state {
                ServerState::Lobby => {
                    // if and only if there is more than 1 person in the game make a timer that goes from 60 to 0, once reaches 0 switches to initgame, report each second to log
                    if let Some(timer) = &lobby_timer {
                        if game.players.len() <= 1 {
                            lobby_timer = None;
                            continue
                        }
                        let elapsed = timer.elapsed();
                        if elapsed > LOBBY_TIME {
                            game.execute_cmd(User::Server, GameCmd::AddLogMessage("Game starting...".into()))?;
                            *self.state.write().await = ServerState::InitGame;
                            lobby_timer = None;
                        }
                        else if lobby_report_interval.check() {
                            let remaining = LOBBY_TIME-elapsed;
                            game.execute_cmd(User::Server, GameCmd::AddLogMessage(format!("Game starts in {} seconds!", remaining.as_secs())))?;
                        }
                    }
                    else {
                        if game.players.len() > 1 {
                            lobby_timer = Some(Instant::now());
                        }
                    }
                },
                ServerState::InitGame => {
                    let mut new_game = Game::new();
                    for (id, _) in &game.players {
                        new_game.execute_cmd(User::Server, GameCmd::AddPlayer(*id))?;
                    }
                    *game = new_game;

                    // create a star base for each player and give them materials
                    for (player_id, _) in game.players.clone() {
                        game.execute_cmd(User::Server, GameCmd::SpawnStarBase(player_id, Vec2::random_unit_circle()*5000., Vec2::ZERO))?;
                        game.execute_cmd(
                            User::Server,
                            GameCmd::GiveMaterials(
                                player_id,
                                vec![
                                    (Material::Iron, 1000.),
                                    (Material::Nickel, 1000.),
                                    (Material::Silicates, 1000.),
                                    (Material::Copper, 1000.),
                                    (Material::Carbon, 1000.),
                                ]
                                .into_iter()
                                .collect(),
                            ),
                        )?
                        
                    }

                    for _ in 0..300 {
                        game.execute_cmd(User::Server, GameCmd::SpawnRandomAsteroid(Vec2::random_unit_circle()*10000., Vec2::random_unit_circle()*10.))?;
                    }

                    game_timer = Some(Instant::now());
                    *self.state.write().await = ServerState::Game;
                },
                ServerState::Game => {
                    // track events, if someone's base gets destroyed report it and also destroy every game object of that player

                    for event in game.events.clone() {
                        if let GameEvent::GameObjectDestroyed(GameObject::StarBase(sb), destroyer) = event {
                            let destroyed_id = sb.owner;
                            let destroyer_name = if let GameObject::Asteroid(_) = destroyer { "an asteroid".into() } else {format!("player {}", destroyer.owner().unwrap())};
                            game.execute_cmd(User::Server, GameCmd::AddLogMessage(format!("Player {destroyed_id} got annihilated by {}", destroyer_name)))?;
                        }
                    }

                    if game.star_bases().len() == 0 {
                        game.execute_cmd(User::Server, GameCmd::AddLogMessage("No one survived, it is a draw!".into())).unwrap();
                        *self.state.write().await = ServerState::Lobby;
                    }
                    else if game_timer.unwrap().elapsed() > GAME_TIME {
                        game.execute_cmd(User::Server, GameCmd::AddLogMessage("Game time has ran out, it is a draw!".into())).unwrap();
                        *self.state.write().await = ServerState::Lobby;
                    }
                    else if game.star_bases().len() == 1 {
                        let winner_id = game.star_bases()[0].owner;
                        game.execute_cmd(User::Server, GameCmd::AddLogMessage(format!("Winner is player {}!", winner_id)))?;
                        *self.state.write().await = ServerState::Lobby;
                    }
                },
            }

            let dt = now() - game.sync.last_update;
            game.update(dt.as_secs_f32());

            drop(game);


            let frame_time = frame_time_measure.elapsed();
            if frame_time < target_duration {
                tokio::time::sleep(target_duration - frame_time).await;
            } else {
                eprintln!(
                    "Server is behind intended frame rate, delay: {} ms",
                    (frame_time - target_duration).as_millis()
                );
                yield_now().await;
            }
        }
    }
}

async fn handle_connections(game: Arc<RwLock<Game>>, hub_conn: ServerHubConn, server_state: Arc<RwLock<ServerState>>) {
    let hub_conn = Arc::new(hub_conn);
    // keep alive task
    {
        let hub_conn_c = hub_conn.clone();
        tokio::spawn(async move {
            let hub_conn = hub_conn_c;
            let mut ka_interval = interval(Duration::from_secs(60*3));
            loop {
                ka_interval.tick().await;
                println!("Sending keep alive!");
                hub_conn.keep_alive().await;
            }
        });
    }

    let listener = TcpListener::bind(format!("0.0.0.0:{}", SERVER_ADDR.port())).await.unwrap();
    println!("Listening on address {}", SERVER_ADDR);
    while let Ok((stream, _)) = listener.accept().await {
        println!("Detected potential client");
        let game = game.clone();
        let hub_conn_c = hub_conn.clone();
        let server_state_c = server_state.clone();
        tokio::task::spawn(handle_client(stream, game, hub_conn_c, server_state_c));
    }
}

async fn handle_client(stream: TcpStream, game: Arc<RwLock<Game>>, hub_conn: Arc<ServerHubConn>, server_state: Arc<RwLock<ServerState>>) {
    let mut client_handle = ClientHandle::new(stream, game.clone(),  server_state, hub_conn).await.unwrap();
    loop {
        if let Err(err) = client_handle.update().await {
            eprintln!("User has disconnected with error: {:?}", err);
            if let User::Player(id) = client_handle.user {
                eprintln!("Removing player {} from the game", id);
                game.write().await.execute_cmd(User::Server, GameCmd::RemovePlayer(id)).unwrap();
            }
            return;
        };
    }
}
