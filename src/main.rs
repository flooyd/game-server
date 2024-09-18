use bincode;
use message_io::network::{NetEvent, Transport};
use message_io::node::{self, NodeHandler};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Serialize, Deserialize, Debug)]
enum ClientMessage {
    PlayerPosition { id: usize, x: f32, y: f32 },
    AssignPlayerId { id: usize },
}

#[derive(Clone)]
struct Player {
    id: usize,
    endpoint: message_io::network::Endpoint,
    x: f32,
    y: f32,
}

struct GameState {
    players: RwLock<HashMap<usize, Player>>,
}

fn main() {
    let (handler, listener) = node::split::<()>();
    let game_state = Arc::new(GameState {
        players: RwLock::new(HashMap::new()),
    });
    let mut next_player_id = 1;

    handler
        .network()
        .listen(Transport::Tcp, "0.0.0.0:3042")
        .unwrap();

    let handler_clone = handler.clone();
    let game_state_clone = Arc::clone(&game_state);

    listener.for_each(move |event| match event.network() {
        NetEvent::Connected(_, _) => unreachable!(),
        NetEvent::Accepted(endpoint, _) => {
            println!("Client connected: {:?}", endpoint);
            let player = Player {
                id: next_player_id,
                endpoint,
                x: 0.0,
                y: 0.0,
            };
            game_state_clone
                .players
                .write()
                .unwrap()
                .insert(next_player_id, player.clone());

            let assign_msg = ClientMessage::AssignPlayerId { id: next_player_id };
            let message = bincode::serialize(&assign_msg).unwrap();
            handler_clone.network().send(endpoint, &message);
            next_player_id += 1;
        }
        NetEvent::Message(endpoint, data) => {
            let message: ClientMessage = match bincode::deserialize(&data) {
                Ok(msg) => msg,
                Err(e) => {
                    println!("Deserialization error: {:?}", e);
                    return;
                }
            };
            match message {
                ClientMessage::PlayerPosition { id, x, y } => {
                    // Update server's game state
                    if let Some(player) = game_state_clone.players.write().unwrap().get_mut(&id) {
                        player.x = x;
                        player.y = y;
                    }

                    // Broadcast to all other players
                    let players = game_state_clone.players.read().unwrap();
                    let broadcast_data =
                        bincode::serialize(&ClientMessage::PlayerPosition { id, x, y }).unwrap();
                    for (pid, player) in players.iter() {
                        if *pid != id {
                            handler_clone
                                .network()
                                .send(player.endpoint, &broadcast_data);
                        }
                    }
                }
                ClientMessage::AssignPlayerId { id } => {
                    println!("Unexpected AssignPlayerId message from client: {}", id);
                }
            }
        }
        NetEvent::Disconnected(endpoint) => {
            println!("Client disconnected: {:?}", endpoint);
            let mut players = game_state_clone.players.write().unwrap();
            if let Some((id, _)) = players
                .iter()
                .find(|(_, p)| p.endpoint == endpoint)
                .map(|(k, v)| (*k, v.clone()))
            {
                players.remove(&id);
                println!("Removed player with ID: {}", id);
            }
        }
    });
}
