use clap::{Parser};
use core::cli::complete::{Complete, print_completions};
use core::cli::{Cli, Commands, ls, pf, agent};
use core::endpoint::{docker::DockerEndpoint, kube::KubeConfigs};
use env_logger::Builder;


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {

    
    
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        let kube = KubeConfigs::new();
        let docker = DockerEndpoint::new();
        match args[1].as_str() {
            "-cb" => {
                let comp = Complete::new(kube, docker);
                comp.bash_complete().await;
                return Ok(());
            },
            _ => {}
        }
    }
    
    let cli = Cli::parse();
    
    if cli.verbose {
        Builder::new()
            .parse_filters("info")
            .init();
    }
    let kube = KubeConfigs::new();
    let docker = DockerEndpoint::new();
    return match cli.command {
        Some(Commands::Ls { endpoint }) => {
            let ls = ls::Ls::new(kube, docker);
            ls.exec(endpoint).await
        },
        Some(Commands::Pf { origin, dst }) => {
            let pf = pf::Pf::new(kube);
            pf.exec(origin, dst).await
        },
        Some(Commands::Agent { listen, port }) => {
            let agent = agent::Agent::new(port, listen);
            agent.exec().await;
            Ok(())
        },
        Some(Commands::Completion { shell }) => {
            print_completions(shell);
            Ok(())
        },
        _ => {Ok(())},
    };
}



