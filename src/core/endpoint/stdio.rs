
use std::{os::unix::prelude::FromRawFd, net::SocketAddr, process::{exit, Stdio}, path::Path, str::FromStr};

use tokio::{io::{stdin, stdout, AsyncReadExt}, fs::File, process::Command};

use super::{PipeEndpoint, socket::TCPConnectionProvider, connect, PipeCopyDestination, PipeCopySource};
use crate::mux::Multiplexer;

pub struct StdioPipeEndpoint;

#[derive(Debug, Clone)]
pub enum Error {
    PathDoesNotExist,
    ExecError(String)
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::ExecError(e.to_string())
    }
}

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

pub async fn local_ls(path: &str) -> Vec<String> {
    let com = Command::new("ls")
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn().expect("Failed to spawn ls");
    
    let mut stdout = com.stdout.expect("Faield to get ls output");
    let mut buf = String::new();
    stdout.read_to_string(&mut buf).await.expect("Failed to read ls output");
    
    buf.split_whitespace().map(String::from).collect()
}

pub async fn get_copy_destination(path: String) -> Result<PipeCopyDestination, Error> {
    let os_path = Path::new(path.as_str());
    if !os_path.is_dir() {
        return Err(Error::PathDoesNotExist)
    }
    let com = Command::new("tar")
        .arg("xf")
        .arg("-")
        .arg("-C")
        .arg(path)
        .stdin(Stdio::piped())
        .spawn()?;

    let stdin = com.stdin.ok_or(Error::ExecError("Failed tar pipe".to_owned()))?;
    Ok(PipeCopyDestination::new(Box::new(stdin)))
}

pub async fn get_copy_source(path: String) -> Result<PipeCopySource, Error> {
    let path_parts = Path::new(path.as_str());
    let target: &str;
    let dir: &str;
    if path.ends_with("/") {
        target = ".";
        dir = path.strip_suffix("/").unwrap();
    } else {
        target = path_parts.file_name().unwrap().to_str().unwrap();
        dir = path_parts.parent().unwrap().to_str().unwrap(); 
    };
    if !path_parts.exists() {
        return Err(Error::PathDoesNotExist)
    }
    let size_com = Command::new("sh")
        .arg("-c")
        .arg(format!("tar cf - -C {} {} | wc -c", dir, target))
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    let mut stdout_buf = vec![];
    let mut stderr_buf = vec![];
    let mut du_stdout = size_com.stdout.expect("Failed to get wc stdout");
    let mut du_stderr = size_com.stderr.expect("Failed to get wc stderr");

    du_stderr.read_to_end(&mut stderr_buf).await.or(Err(Error::ExecError("Failed to exec wc".to_owned())))?;
    let e = std::str::from_utf8(&mut stderr_buf).expect("Failed to parse return");
    if !e.is_empty() {
        return Err(Error::ExecError(e.to_owned()));
    }

    du_stdout.read_to_end(&mut stdout_buf).await.or(Err(Error::ExecError("Failed to exec du".to_owned())))?;
    let mut du_res = String::from_utf8_lossy(&stdout_buf).to_string();
    du_res.retain(|s| s.is_numeric());
    let size: u64 = FromStr::from_str(du_res.as_str()).expect("Failed to parse file size");
    
    let com = Command::new("tar")
        .arg("cf")
        .arg("-")
        .arg("-C")
        .arg(dir)
        .arg(target)
        .stdout(Stdio::piped())
        .spawn()?;
    let out = com.stdout.ok_or(Error::ExecError("Failed tar pipe".to_owned()))?;
    Ok(PipeCopySource::new(size, Box::new(out)))
}





