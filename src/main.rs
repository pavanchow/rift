use clap::{Parser, Subcommand};
use std::net::SocketAddr;

#[derive(Parser)]
#[command(name = "rift", version, about = "A programmable single-binary reverse proxy where routing rules are a compiled DSL, not YAML.")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the proxy with a rules file.
    Serve {
        /// Path to the routing DSL file.
        #[arg(long)]
        config: String,
        /// Address to listen on.
        #[arg(long, default_value = "127.0.0.1:8080")]
        addr: SocketAddr,
    },
    /// Parse and validate a rules file, then exit.
    Check {
        #[arg(long)]
        config: String,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Check { config } => {
            let src = std::fs::read_to_string(&config)?;
            match rift::rules::parse(&src) {
                Ok(router) => {
                    println!("ok: {} rule(s)", router.rules.len());
                    Ok(())
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        }
        Cmd::Serve { config, addr } => {
            let src = std::fs::read_to_string(&config)?;
            let router = rift::rules::parse(&src).map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(rift::server::serve(router, addr))
        }
    }
}
