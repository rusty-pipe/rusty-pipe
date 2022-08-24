use futures::future::join_all;
use tokio::io::{AsyncWrite, AsyncRead, copy, split};
use tokio::sync::oneshot;
use tokio::{task, select};

pub trait PipeEndpoint: Send + Unpin {
    fn get_sink_and_source(self: Box<Self>) -> (Box<dyn AsyncRead + Unpin + Send + 'static>, Box<dyn AsyncWrite + Unpin + Send + 'static>);
}

pub async fn connect(from: Box<dyn PipeEndpoint>, to: Box<dyn PipeEndpoint>) -> u64
{
    let (mut source1, mut sink1) = Box::new(from).get_sink_and_source();
    let (mut source2, mut sink2) = Box::new(to).get_sink_and_source();
    let (kill1, end1) = oneshot::channel::<bool>();
    let (kill2, end2) = oneshot::channel::<bool>();
    
    let t1 = task::spawn(async move {
        select! {
            size = copy(&mut source1, &mut sink2) => {
                _ = kill2.send(true);
                return match size {
                    Ok(s) => s,
                    Err(_) => 0  
                };
            },
            _ = end1 => {
                return 0;
            }
        };
    });
    
    let t2 = task::spawn(async move {
        select! {
            size = copy(&mut source2, &mut sink1) => {
                _ = kill1.send(true);
                return match size {
                    Ok(s) => s,
                    Err(_) => 0  
                };
            },
            _ = end2 => {
                return 0;
            }
        };
    });

    let res = join_all(vec![t1, t2]).await;
    let size1 = match res[0] {
        Ok(s) => s,
        Err(_) => 0,  
    };
    let size2 = match res[1] {
        Ok(s) => s,
        Err(_) => 0,  
    };
    if size1 > size2 {
        return size1.clone();
    }
    size2.clone()
}

impl<T> PipeEndpoint for T 
    where T : AsyncRead + AsyncWrite + Unpin + Send + 'static
{
    fn get_sink_and_source(self: Box<Self>) -> (Box<(dyn tokio::io::AsyncRead + Unpin + std::marker::Send + 'static)>, Box<(dyn tokio::io::AsyncWrite + Unpin + std::marker::Send + 'static)>) {
        let (read, write) = split(self);
        return (Box::new(read), Box::new(write));
    }
}

pub static AGENT: &[u8] = include_bytes!("../../target/release/agent");
pub static AGENT_PATH: &str = "/tmp/rs-agent";
pub static AGENT_KILL_PATH: &str = "/tmp/rs-agent.kill";

pub mod stdio;

pub mod socket;

pub mod kube;

pub mod docker;
