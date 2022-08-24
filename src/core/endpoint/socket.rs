
use std::{net::SocketAddr, process::exit};

use tokio::{net::{TcpListener, TcpSocket, TcpStream}};
pub struct TCPConnectionProvider {
    address: SocketAddr
}

impl TCPConnectionProvider {
    pub fn new(address: SocketAddr) -> Self {
        TCPConnectionProvider { address: address }
    }

    pub async fn listen_for_connections(self) -> TcpListener {
        match TcpListener::bind(self.address).await {
            Ok(l) => l,
            Err(e) => {
                eprintln!("Error: {e}");
                exit(1);
            },
        }
    }
    
    pub async fn connect(self) -> TcpStream {
        let soc = TcpSocket::new_v4().unwrap();
        match soc.connect(self.address).await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Error: {e}");
                exit(1);
            },
        }
    } 
}
