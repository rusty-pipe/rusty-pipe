
use clap::{ErrorKind, CommandFactory};
use tokio::{net::TcpSocket};

use crate::endpoint::{kube::KubeConfigs, AGENT_PATH, PipeEndpoint, self, stdio::StdioPipeEndpoint, connect, socket::TCPConnectionProvider, docker::DockerEndpoint};

use super::{ForwardPoint, Cli, KubeForwardPoint, DockerForwardPoint};

pub struct Pf {
    kube: KubeConfigs
}

enum Error {
    Docker(endpoint::docker::Error),
    Kube(endpoint::kube::Error)
}

impl From<endpoint::docker::Error> for Error {
    fn from(e: endpoint::docker::Error) -> Self {
        Error::Docker(e)
    }
}

impl From<endpoint::kube::Error> for Error{
    fn from(e: endpoint::kube::Error) -> Self {
        Error::Kube(e)
    }
}


impl Pf {
    pub fn new(kube: KubeConfigs) -> Pf {
        Pf {kube}
    }

    pub async fn exec(&self, origin: ForwardPoint, dst: ForwardPoint) -> Result<(), Box<dyn std::error::Error>> { 
        let mut cmd = Cli::command();
        if matches!(origin, ForwardPoint::Stdio) && matches!(dst, ForwardPoint::Stdio) {
            cmd.error(ErrorKind::ArgumentConflict, "Both forward points could not be STDIO").exit();
        }
        match accept_origin_endpoint(origin, dst, &self.kube).await {
            Ok(_) => {},
            Err(_) => {
                cmd.error(ErrorKind::Io, "Port forward failed");
            },
        }
        Ok(())
    }
}

async fn get_destination_endpoint(p: ForwardPoint, kube: &KubeConfigs) -> Result<Box<dyn PipeEndpoint>, Error> {
    match p {
        ForwardPoint::Kube(k) => {
            let KubeForwardPoint{context, namespace, pod, port} = k;
            let pf = kube.get_port_forward(context, namespace, pod, port).await?;
            return Ok(Box::new(pf));
        },
        ForwardPoint::Stdio => {
            return Ok(Box::new(StdioPipeEndpoint {}));
        },
        ForwardPoint::Local(l) => {
            let soc = TcpSocket::new_v4().expect("Failed to create local connection");
            let con = soc.connect(l).await.expect(&format!("Failed to connect to {l}"));
            return Ok(Box::new(con));
        },
        ForwardPoint::Docker(DockerForwardPoint{port, container}) => {
            let doc = DockerEndpoint::new();
            let agent_exec = vec![AGENT_PATH.to_string(), "agent".to_string(), "-p".to_string(), port.to_string()];
            doc.install_agent(&container).await?;
            let con = doc.connect(&container, agent_exec).await?;
            return Ok(Box::new(con));
        },
    };
}

async fn accept_origin_endpoint(origin: ForwardPoint, destination: ForwardPoint, kube: &KubeConfigs) -> Result<(), Error> {
    match origin {
        ForwardPoint::Kube(k) => {
            let KubeForwardPoint{context, namespace, pod, port} = k;
            let agent_exec = vec![AGENT_PATH.to_string(), "agent".to_string(), "-l".to_string(), "-p".to_string(), port.to_string()];
            kube.install_agent(context.clone(), namespace.clone(), pod.clone()).await?;
            let mut rec = kube.get_connections(context, namespace, pod, &agent_exec).await?;
            while let Some(con) = rec.recv().await {
                let to = get_destination_endpoint(destination.clone(), kube).await?;
                tokio::spawn(async {
                    connect(Box::new(con), to).await;
                });
            }
        },
        ForwardPoint::Stdio => {
            let from = StdioPipeEndpoint{};
            let to = get_destination_endpoint(destination, kube).await?;
            connect(Box::new(from), to).await;
        },
        ForwardPoint::Local(addr) => {
            let provider = TCPConnectionProvider::new(addr).listen_for_connections().await;
            while let Ok((con, _)) = provider.accept().await {
                let to = get_destination_endpoint(destination.clone(), kube).await?;
                tokio::spawn(async {
                    connect(Box::new(con), to).await;
                });
            }
        },
        ForwardPoint::Docker(DockerForwardPoint{port, container}) => {
            let doc = DockerEndpoint::new();
            let agent_exec = vec![AGENT_PATH.to_string(), "agent".to_string(), "-l".to_string(), "-p".to_string(), port.to_string()];
            doc.install_agent(&container).await?;
            let mut rec = doc.get_connections(&container, agent_exec).await?;
            while let Some(con) = rec.recv().await {
                let to = get_destination_endpoint(destination.clone(), kube).await?;
                tokio::spawn(async {
                    connect(Box::new(con), to).await;
                });
            }
        },
    };
    Ok(())
}