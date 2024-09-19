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
    UpdateMessage { id: usize, message: String },
    OtherPlayerConnected { id: usize, x: f32, y: f32 },
}

#[derive(Clone)]
struct Player {
    id: usize,
    endpoint: message_io::network::Endpoint,
    x: f32,
    y: f32,
    message: String,
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
        .listen(Transport::FramedTcp, "0.0.0.0:3042")
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
                message: String::new(),
            };
            game_state_clone
                .players
                .write()
                .unwrap()
                .insert(next_player_id, player);
            let message =
                bincode::serialize(&ClientMessage::AssignPlayerId { id: next_player_id }).unwrap();
            handler_clone.network().send(endpoint, &message);

            //send
            next_player_id += 1;
        }
        
        NetEvent::Message(endpoint, data) => {
            let message: ClientMessage = bincode::deserialize(&data).unwrap();
            match message {
                ClientMessage::PlayerPosition { id, x, y } => {
                    // Update the player's position in the game state
                    println!("Player position: {:?}", (id, x, y));
                    let mut players = game_state_clone.players.write().unwrap();
                    if let Some(player) = players.get_mut(&id) {
                        player.x = x;
                        player.y = y;
                    }

                    // Broadcast the message to all other players
                    let broadcast_data =
                        bincode::serialize(&ClientMessage::PlayerPosition { id, x, y }).unwrap();
                    broadcast_message(&handler_clone, &players, &broadcast_data, id);
                }
                ClientMessage::UpdateMessage { id, message } => {
                    // Update the player's message in the game state
                    let message_start_time = std::time::Instant::now();
                    let mut players = game_state_clone.players.write().unwrap();
                    if let Some(player) = players.get_mut(&id) {
                        player.message = message.clone();
                    }

                    // Broadcast the updated message to all players
                    let broadcast_data =
                        bincode::serialize(&ClientMessage::UpdateMessage { id, message }).unwrap();
                    broadcast_message(&handler_clone, &players, &broadcast_data, id);
                    println!("Message processing time: {:?}", message_start_time.elapsed());
                }
                ClientMessage::AssignPlayerId { id } => todo!(),
                ClientMessage::OtherPlayerConnected { id, x, y } => {}
            }
        }
        NetEvent::Disconnected(endpoint) => {
            println!("Client disconnected: {:?}", endpoint);
            let mut players = game_state_clone.players.write().unwrap();
            players.retain(|_, player| player.endpoint != endpoint);
        }
    });
}

// Function to broadcast a message to all connected clients except the sender
fn broadcast_message(
    handler: &NodeHandler<()>,
    players: &HashMap<usize, Player>,
    data: &[u8],
    sender_id: usize,
) {
    for player in players.values() {
        if player.id != sender_id {
            handler.network().send(player.endpoint, data);
        }
    }
}
