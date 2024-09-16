use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Debug,  PartialEq, Clone)]
enum Message {
    Move {
        player_id: Uuid,
        x: f32,
        y: f32,
    },
    PlayerMap {
        player_map: HashMap<String, PlayerMapPlayer>,
    },
    Disconnect {
        player_id: Uuid,
    },
    Connect {
        player_id: Uuid,
    },
    OtherPlayerConnected {
        player: PlayerMapPlayer,
    },
    GetPlayerMap,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct PlayerMapPlayer {
    player_id: Uuid,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    color: [f32; 4],
}

impl PartialEq for PlayerMapPlayer {
    fn eq(&self, other: &Self) -> bool {
        self.player_id == other.player_id
            && self.x.to_bits() == other.x.to_bits()
            && self.y.to_bits() == other.y.to_bits()
            && self.width.to_bits() == other.width.to_bits()
            && self.height.to_bits() == other.height.to_bits()
            && self
                .color
                .iter()
                .zip(other.color.iter())
                .all(|(a, b)| a.to_bits() == b.to_bits())
    }
}

impl Eq for PlayerMapPlayer {}

#[tokio::main]
async fn main() -> tokio::io::Result<()> {
    let listener = TcpListener::bind("0.0.0.0:8080").await?;
    println!("Server running on {}", listener.local_addr()?);

    // Shared player map and clients map
    let player_map = Arc::new(Mutex::new(HashMap::new()));
    let clients = Arc::new(Mutex::new(HashMap::new()));

    loop {
        let (socket, addr) = listener.accept().await?;
        let addr_str = addr.to_string();
        println!("Player connected: {}", addr_str);

        let player_id = Uuid::new_v4();
        let player = PlayerMapPlayer {
            player_id,
            
            //random from 0 to 400
            x: rand::random::<f32>() * 400.0,
            y: rand::random::<f32>() * 400.0,
            width: 10.0,
            height: 10.0,
            //random color
            color: [rand::random::<f32>(), rand::random::<f32>(), rand::random::<f32>(), 1.0],
        };

        // Add player to the player map
        player_map
            .lock()
            .unwrap()
            .insert(addr_str.clone(), player.clone());

        // Create an unbounded channel for this client
        let (tx, rx) = mpsc::unbounded_channel::<Message>();

        // Add the sender to the clients map
        clients.lock().unwrap().insert(addr_str.clone(), tx.clone());

        // Send the player_id to the new client
        let initial_message = Message::Connect { player_id };
        tx.send(initial_message).unwrap();

        let other_player_connected_message = Message::OtherPlayerConnected { player };
        broadcast_message(&clients, &addr_str, other_player_connected_message);

        // Clone references for the task
        let clients_clone = Arc::clone(&clients);
        let player_map_clone = Arc::clone(&player_map);

        // Split the socket into read and write halves
        let (reader, writer) = socket.into_split();

        // Spawn a task to handle client communication
        tokio::spawn(handle_client(
            reader,
            writer,
            addr_str.clone(),
            clients_clone,
            player_map_clone,
            rx,
        ));
    }
}

// Function to broadcast a message to all clients except the sender
fn broadcast_message(
    clients: &Arc<Mutex<HashMap<String, mpsc::UnboundedSender<Message>>>>,
    sender_addr: &String,
    message: Message,
) {
    let clients_lock = clients.lock().unwrap();
    for (addr, tx) in clients_lock.iter() {
        if addr != sender_addr {
            if let Err(e) = tx.send(message.clone()) {
                eprintln!("Failed to send message to {}: {}", addr, e);
            }
        }
    }
}

async fn handle_client(
    mut reader: tokio::net::tcp::OwnedReadHalf,
    mut writer: tokio::net::tcp::OwnedWriteHalf,
    addr: String,
    clients: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<Message>>>>,
    player_map: Arc<Mutex<HashMap<String, PlayerMapPlayer>>>,
    mut rx: mpsc::UnboundedReceiver<Message>,
) {
    let mut read_buffer = [0u8; 8192];

    // Clone references for the tasks
    let clients_read_task = Arc::clone(&clients);
    let player_map_read_task = Arc::clone(&player_map);

    // Task to read messages from the client (optional for your use case)
    let addr_clone = addr.clone();
    let read_task = tokio::spawn(async move {
        loop {
            match reader.read(&mut read_buffer).await {
                Ok(0) => {
                    // Connection closed by client
                    println!("Connection closed by client: {}", addr_clone);
                    break;
                }
                Ok(_n) => {
                    let message_str = String::from_utf8_lossy(&read_buffer).trim_end_matches(char::from(0)).to_string();
                    let message: Message = serde_json::from_str(&message_str).unwrap();
                    match message {
                        Message::GetPlayerMap => {
                            let player_map_message = Message::PlayerMap {
                                player_map: player_map_read_task.lock().unwrap().clone(),
                            };
                            if let Err(e) = clients_read_task.lock().unwrap().get(&addr_clone).unwrap().send(player_map_message) {
                                eprintln!("Failed to send player map to client {}: {}", addr_clone, e);
                            }
                        }
                        Message::Move { player_id, x, y } => {
                            let mut player_map_lock = player_map_read_task.lock().unwrap();
                            let player = player_map_lock.get_mut(&addr_clone).unwrap();
                            player.x = x;
                            player.y = y;

                            let move_message = Message::Move { player_id, x, y };
                            broadcast_message(&clients_read_task, &addr_clone, move_message);
                        }
                        _ => (),
                    }
                    read_buffer.fill(0);
                }
                Err(e) => {
                    eprintln!("Failed to read from socket {}: {}", addr_clone, e);
                    break;
                }
            }
        }
    });

    // Task to send messages to the client
    let addr_clone = addr.clone();
    let write_task = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            let serialized = serde_json::to_string(&message).unwrap();
            if let Err(e) = writer.write_all(serialized.as_bytes()).await {
                eprintln!("Failed to send message to client {}: {}", addr_clone, e);
                break;
            }
        }
    });

    // Wait for either task to finish
    tokio::select! {
        _ = read_task => (),
        _ = write_task => (),
    }

    let clients_clone = Arc::clone(&clients);
    let player_map_clone = Arc::clone(&player_map);
    let player_id = player_map_clone.lock().unwrap().get(&addr).unwrap().player_id;

    // Clean up after client disconnects
    clients_clone.lock().unwrap().remove(&addr);
    player_map.lock().unwrap().remove(&addr);
    println!("Broadcasting disconnect message");
    // Broadcast the updated player map to remaining clients
    let disconnect_message = Message::Disconnect {
        player_id,
    };

    broadcast_message(&clients_clone, &addr, disconnect_message);

    println!("Cleaned up client: {}", addr);
}
