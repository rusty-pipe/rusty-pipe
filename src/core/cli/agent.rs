use std::{net::{SocketAddr, Ipv4Addr, IpAddr}, process::exit, time::Duration};

use tokio::time::sleep;

use crate::{endpoint::{stdio::{multiplex_con_to_stdio, stdio_to_con}, AGENT_KILL_PATH}};

pub struct Agent {
    port: u16,
    listen: bool
}

impl Agent {
    pub fn new(port: u16, listen: bool) -> Agent {
        Agent {port, listen}
    }

    pub async fn exec(&self) {

        

        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), self.port);
        if self.listen {
            // listen for kill signal
            tokio::spawn(async {
                loop {
                    match tokio::fs::File::open(AGENT_KILL_PATH).await {
                        Ok(_) => {
                            tokio::fs::remove_file(AGENT_KILL_PATH).await.unwrap();
                            exit(0);
                        },
                        Err(_) => {},
                    };
                    sleep(Duration::from_millis(1000)).await;
                }
            });  
           multiplex_con_to_stdio(addr).await;
           return;
        }
        stdio_to_con(addr).await;
    }
}