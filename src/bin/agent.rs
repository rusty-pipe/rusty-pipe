use clap::Parser;
use core::cli::{Cli, Commands, agent::Agent};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match cli.command {
        Some(Commands::Agent { port, listen }) => {
            let agent = Agent::new(port, listen);
            agent.exec().await
        },
        _ => {},
    };
}