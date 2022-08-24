
use std::{os::unix::prelude::FromRawFd, net::SocketAddr, process::exit};

use tokio::{io::{stdin, stdout}, fs::File};

use super::{PipeEndpoint, socket::TCPConnectionProvider, connect};
use crate::mux::Multiplexer;

pub struct StdioPipeEndpoint;

impl PipeEndpoint for StdioPipeEndpoint {
    fn get_sink_and_source(self: Box<Self>) -> (Box<(dyn tokio::io::AsyncRead + Unpin + std::marker::Send + 'static)>, Box<(dyn tokio::io::AsyncWrite + Unpin + std::marker::Send + 'static)>) {
        return (Box::new(stdin()), Box::new(stdout()));
    }
}

pub async fn multiplex_con_to_stdio(addr: SocketAddr) {
    let socket = TCPConnectionProvider::new(addr).listen_for_connections().await;
    let in_buffer = unsafe { File::from_raw_fd(0) }; //stdin
    let out_buffer = unsafe { File::from_raw_fd(1) }; //stdout
    let mux = Multiplexer::new().consume_connections(in_buffer, out_buffer);
    while let Ok((con, _)) = socket.accept().await {
        match mux.send(con).await {
            Ok(_) => {},
            Err(e) => {
                log::error!("{}", e);
                return;
            }
        }
    }
}

pub async fn stdio_to_con(addr: SocketAddr) {
    let soc = TCPConnectionProvider::new(addr).connect().await;
    connect(Box::new(soc), Box::new(StdioPipeEndpoint{})).await;
    exit(0);
}




