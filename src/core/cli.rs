use std::net::{SocketAddr, IpAddr, Ipv4Addr};

use clap::{Parser, Subcommand, AppSettings, ValueEnum};



/// Rusty pipe - quick and rusty tool to port forward or reverse port forward between localhost, containers and pods
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct Cli {

    #[clap(short='v', value_parser)]
    pub verbose: bool,

    #[clap(subcommand)]
    pub command: Option<Commands>,
    
}

static FORWARD_POINT_HELP: &str = 
"
Available forward points are:
    Kubernetes: '<context>/<namespace>/<pod>:<PORT>'
    Docker: '<container>:<PORT>'
    Local: '[ADDR]:<PORT>'
    STDIO: '-'
";

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Shell {
    Bash
}


#[derive(Subcommand, Debug)]
pub enum Commands {
    /// List available endpoints
    Ls {
        /// endpoint string or it's part
        #[clap(value_parser=str_to_ls_path)]
        endpoint: Option<String>
    },
    
    /// Port forward from ORIGIN to DESTINATION
    Pf {
        /// Originating endpoint
        #[clap(value_parser=str_to_forward_point, name="ORIGIN", long_help=FORWARD_POINT_HELP)]
        origin: ForwardPoint,

        /// Destination endpoint
        #[clap(value_parser=str_to_forward_point, name="DESTINATION", long_help=FORWARD_POINT_HELP)]
        dst: ForwardPoint,
    },
    
    /// Output shell completion code
    Completion {
        /// Shell 
        #[clap(value_parser, arg_enum)]
        shell: Shell
    },
    
    /// Stdio agent
    #[clap(setting = AppSettings::Hidden)]
    Agent {
        /// Listen for connections
        #[clap(short='l', long, value_parser)]
        listen: bool,

        /// Local port number
        #[clap(short='p', long, value_parser)]
        port: u16
    },

}


#[derive(Debug, Clone)]
pub enum ForwardPoint {
    Docker(DockerForwardPoint),
    Kube(KubeForwardPoint),
    Local(SocketAddr),
    Stdio,
}

#[derive(Debug, Clone)]
pub struct KubeForwardPoint {
    pub context: String,
    pub namespace: String,
    pub pod: String,
    pub port: u16
}
#[derive(Debug, Clone)]
pub struct DockerForwardPoint {
    pub container: String,
    pub port: u16,
}

fn str_to_ls_path(val: &str) -> Result<String, String> {
    let err = "path must follow <context>/[ns/[pod:/]] OR <container>:/".to_string();
    if !val.contains(":") && !val.ends_with("/") {
        return Err(err);
    }
    return Ok(val.to_string());
}

fn str_to_forward_point(val: &str) -> Result<ForwardPoint, String> {
    let parts: Vec<String> = val.split("/").map(|p| {String::from(p)}).collect();
    match parts.len() {
        1 => {
            if parts[0] == "-" {
                return Ok(ForwardPoint::Stdio);
            }
            if parts[0].contains(":") {
                let host_and_port: Vec<String> = parts[0].split(":").map(|p| {String::from(p)}).collect();
                let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
                let port = host_and_port[1].parse::<u16>().or(Err("Invalid port"))?;
                if host_and_port[0].is_empty() {
                    return Ok(ForwardPoint::Local(
                        SocketAddr::new(ip, port)
                    ));
                }
                
                if host_and_port[0].eq("localhost") || host_and_port[0].eq("127.0.0.1") {
                    return Ok(ForwardPoint::Local(
                        SocketAddr::new(ip, port)
                    ));
                }
                if host_and_port[0].matches(".").count() == 3 {
                    let addr = parts[0].parse::<SocketAddr>().or(Err("Invalid address"))?;
                    return Ok(ForwardPoint::Local(addr));
                }
                return Ok(ForwardPoint::Docker(DockerForwardPoint{container: host_and_port[0].clone(), port }));
            }
            Err("Missing :<PORT> part".to_string())
        },
        3 => {
            if parts[2].contains(":") {
                let pod_port: Vec<String> = parts[2].split(":").map(|p| {String::from(p)}).collect();
                return Ok(ForwardPoint::Kube(
                    KubeForwardPoint {
                        context: parts[0].clone(),
                        namespace: parts[1].clone(),
                        pod: pod_port[0].clone(),
                        port: pod_port[1].parse::<u16>().unwrap(),
                    }
                ));
            }
            
            Err("Missing :<PORT> part".to_string())
        },
        _ => {
            Err("Not a valid forward path".to_string())
        }
    }
}

pub mod ls;
pub mod pf;
pub mod agent;
pub mod complete;