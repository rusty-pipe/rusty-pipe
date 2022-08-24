use bollard::{Docker, container::{ListContainersOptions, LogOutput}, exec::{CreateExecOptions, StartExecResults}};
use tokio::{sync::{mpsc::{Receiver}, oneshot}, io::{AsyncRead, AsyncWrite, duplex, stderr, AsyncWriteExt, split, copy}};
use futures::StreamExt;
use std::{default::Default};

use crate::mux::Multiplexer;

use super::{AGENT, AGENT_PATH, AGENT_KILL_PATH};

#[derive(Debug, Clone)]
pub enum Error {
    FailedToInitDocker,
    DockerError(String)
}



impl From<bollard::errors::Error> for Error {
    fn from(e: bollard::errors::Error) -> Self {
       Error::DockerError(e.to_string())
    }
}


pub struct DockerEndpoint {
    docker: Result<Docker, Error>
}

impl DockerEndpoint {
    pub fn new() -> DockerEndpoint {
        let docker = Docker::connect_with_socket_defaults().or(Err(Error::FailedToInitDocker));
        DockerEndpoint { docker }
    }

    pub async fn list_containers(&self) -> Result<Vec<String>, Error> {
        let doc = match &self.docker {
            Ok(d) => d,
            Err(e) => return Err(e.clone())
        };
        let options = ListContainersOptions::<String> {
            all: false,
            ..Default::default()
        };
        let containers = doc.list_containers(Some(options)).await?;
        let ids: Vec<String> = containers.iter().map(|s| {
            match &s.names {
                Some(names) => {
                    names[0].trim_start_matches('/').to_string()
                },
                None => {
                    s.id.clone().unwrap()
                },
            }
        }).collect();
        Ok(ids)
    }

    pub async fn get_container_files(&self, container_name: &String, path: &String) -> Result<Vec<String>, Error> {
        let doc = match &self.docker {
            Ok(d) => d,
            Err(e) => return Err(e.clone())
        };
        let config = CreateExecOptions::<String> {
            attach_stdout: Some(true),
            attach_stdin: Some(false),
            cmd: Some(vec!["ls".to_string(), path.clone()]),
            ..Default::default()
        };
        let exec = doc.create_exec(container_name, config).await?;
        let mut out: String = "".to_string();
        if let StartExecResults::Attached { mut output, .. } = doc.start_exec(&exec.id, None).await? {
            while let Some(Ok(LogOutput::StdOut { message })) = output.next().await {
                out.push_str(std::str::from_utf8(&message).unwrap());
            }
        } else {
            return Err(Error::DockerError("Failed to exec ls in docker container".to_string()));
        }
        Ok(out.split("\n").map(|s|{s.to_string()}).collect::<Vec<String>>())
    }

    pub async fn install_agent(&self, container_name: &String) -> Result<(), Error> {
        let doc = match &self.docker {
            Ok(d) => d,
            Err(e) => return Err(e.clone())
        };
        let config = CreateExecOptions::<String> {
            attach_stdin: Some(true),
            cmd: Some(vec!["dd".to_string(), format!("of={}", AGENT_PATH)]),
            ..Default::default()
        };
        let exec = doc.create_exec(container_name, config).await?;
        if let StartExecResults::Attached { mut input, .. } = doc.start_exec(&exec.id, None).await? {
            input.write_all(AGENT).await.unwrap();
            let chmod_config = CreateExecOptions::<String> {
                attach_stdin: Some(false),
                cmd: Some(vec!["chmod".to_string(), "+x".to_string(), AGENT_PATH.to_string()]),
                ..Default::default()
            };
            let chmod_exec = doc.create_exec(container_name, chmod_config).await?;
            doc.start_exec(&chmod_exec.id, None).await?;
            return Ok(());
        } else {
            return Err(Error::DockerError("Failed to exec dd in docker container".to_string()));
        }
    }

    pub async fn get_connections(&self, container_name: &str, agent: Vec<String>) -> Result<Receiver<impl AsyncRead + AsyncWrite>, Error> {
        let doc = match &self.docker {
            Ok(d) => d,
            Err(e) => return Err(e.clone())
        }.to_owned();

        let config = CreateExecOptions::<String> {
            attach_stdout: Some(true),
            attach_stdin: Some(true),
            cmd: Some(agent),
            ..Default::default()
        };
        let exec = doc.create_exec(container_name, config).await?;
        match doc.start_exec(&exec.id, None).await? {
            StartExecResults::Attached { mut output, input} => {
                // split output stream into stdout
                let (mut stdout, out) = duplex(255);
                tokio::spawn(async move {
                        let mut stderr = stderr();
                        while let Some(Ok(msg)) = output.next().await {
                            match msg {
                                LogOutput::StdErr { message } => {
                                    match stderr.write(&message).await {
                                        Ok(_) => {},
                                        Err(_) => {},
                                    };
                                },
                                LogOutput::StdOut { message } => {
                                    match stdout.write(&message).await {
                                        Ok(_) => {},
                                        Err(_) => break,
                                    }
                                },
                                _ => {},
                            };
                    }
                });
                // kill switch
                let cn = container_name.to_owned();
                tokio::spawn(async move {
                    tokio::signal::ctrl_c().await.unwrap();
                    let kill_config = CreateExecOptions::<String> {
                        attach_stdin: Some(false),
                        cmd: Some(vec!["touch".to_string(), AGENT_KILL_PATH.to_string()]),
                        ..Default::default()
                    };
                    let kill_exec = doc.create_exec(&cn, kill_config).await.unwrap();
                    doc.start_exec(&kill_exec.id, None).await.unwrap(); 
                });

                Ok(Multiplexer::new().produce_connections(out, input))
            },
            _ => {
                Err(Error::FailedToInitDocker)
            }
        } 
        
    }

    pub async fn connect(&self, container_name: &str, agent: Vec<String>) -> Result<impl AsyncRead + AsyncWrite, Error> {
        let doc = match &self.docker {
            Ok(d) => d,
            Err(e) => return Err(e.clone())
        };
        let config = CreateExecOptions::<String> {
            attach_stdout: Some(true),
            attach_stdin: Some(true),
            cmd: Some(agent),
            ..Default::default()
        };
        let exec = doc.create_exec(container_name, config).await?;
        match doc.start_exec(&exec.id, None).await? {
            StartExecResults::Attached { mut output, mut input} => {
                // split output stream into stdout
                let (stdio, out) = duplex(255);
                let (mut reader, mut writer) = split(stdio);
                let (kill_send, kill_read) = oneshot::channel::<bool>();
                tokio::spawn(async move {
                        let mut stderr = stderr();
                        loop {
                            let msg = match output.next().await {
                                Some(res) => {
                                    match res {
                                        Ok(m) => m,
                                        Err(_) => break,
                                    }
                                },
                                None => break,
                            };
                            match msg {
                                LogOutput::StdErr { message } => {
                                    match stderr.write(&message).await {
                                        Ok(_) => {},
                                        Err(_) => break,
                                    };
                                },
                                LogOutput::StdOut { message } => {
                                    match writer.write_all(&message).await {
                                        Ok(_) => {},
                                        Err(_) => break,
                                    }
                                },
                                _ => {},
                            };
                    }
                    kill_send.send(true).ok();
                });
                tokio::spawn(async move {
                    tokio::select! {
                        _ = copy(&mut reader, &mut input) => {},
                        _ = kill_read => {}
                    }
                });
                Ok(out)
            },
            _ => {
                Err(Error::FailedToInitDocker)
            }
        } 
        
    }

   

}

