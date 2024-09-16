use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone)]
enum Message {
    Move {
        player_id: Uuid,
        direction: String,
    },
    PlayerMap {
        player_map: HashMap<String, PlayerMapPlayer>,
    },
    Disconnect {
        player_id: Uuid,
        player_map: HashMap<String, PlayerMapPlayer>,
    },
    Connect {
        player_id: Uuid,
    }
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
        self.player_id == other.player_id &&
        self.x.to_bits() == other.x.to_bits() &&
        self.y.to_bits() == other.y.to_bits() &&
        self.width.to_bits() == other.width.to_bits() &&
        self.height.to_bits() == other.height.to_bits() &&
        self.color.iter().zip(other.color.iter()).all(|(a, b)| a.to_bits() == b.to_bits())
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
            x: 10.0,
            y: 10.0,
            width: 10.0,
            height: 10.0,
            color: [0.0, 0.0, 0.0, 1.0],
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

        // Send the initial player map to the new client
        let initial_message = Message::Connect {
            player_id,
        };
        tx.send(initial_message).unwrap();

        // Broadcast the new player to all other clients
        let new_player_message = Message::PlayerMap {
            player_map: player_map.lock().unwrap().clone(),
        };
        broadcast_message(&clients, &addr_str, new_player_message);

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
    let mut read_buffer = [0u8; 1024];

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
                    // Handle incoming messages if needed
                    // For now, we don't process client messages
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

    let player_id = player_map.lock().unwrap().get(&addr).unwrap().player_id;

    // Clean up after client disconnects
    clients.lock().unwrap().remove(&addr);
    player_map.lock().unwrap().remove(&addr);
    println!("Broadcasting disconnect message");
    // Broadcast the updated player map to remaining clients
    let disconnect_message = Message::Disconnect {
        player_id,
        player_map: player_map.lock().unwrap().clone(),
    };
    
    broadcast_message(&clients, &addr, disconnect_message);

    println!("Cleaned up client: {}", addr);
}
