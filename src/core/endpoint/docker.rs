use bollard::{Docker, container::{ListContainersOptions, LogOutput}, exec::{CreateExecOptions, StartExecResults}};
use tokio::{sync::{mpsc::{Receiver}, oneshot}, io::{AsyncRead, AsyncWrite, duplex, stderr, AsyncWriteExt, split, copy}};
use futures::StreamExt;
use std::{default::Default, path::Path, str::FromStr};

use crate::mux::Multiplexer;

use super::{AGENT, AGENT_PATH, AGENT_KILL_PATH, PipeCopySource, PipeCopyDestination};

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
#[cfg_attr(test, faux::create)]
pub struct DockerEndpoint {
    docker: Result<Docker, Error>
}
#[cfg_attr(test, faux::methods)]
impl DockerEndpoint {
    pub fn new() -> Self {
        let docker = Docker::connect_with_socket_defaults().or(Err(Error::FailedToInitDocker));
        DockerEndpoint { docker }
    }

    fn get_docker(&self) -> &Result<Docker, Error> {
        &self.docker
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

    pub async fn get_container_files(&self, container_name: &str, path: &str, mut flags: Vec<String>) -> Result<Vec<String>, Error> {
        let doc = match &self.docker {
            Ok(d) => d,
            Err(e) => return Err(e.clone())
        };
        let mut cmd = vec!["ls".to_string(), path.to_owned()];
        cmd.append(&mut flags);
        let config = CreateExecOptions::<String> {
            attach_stdout: Some(true),
            attach_stdin: Some(false),
            cmd: Some(cmd),
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

    
    

    pub async fn get_copy_source(&self, container: &str, path: &str) -> Result<PipeCopySource, Error> {
        let path_parts = Path::new(path);
        let target: &str;
        let dir: &str;
        if path.ends_with("/") {
            target = ".";
            dir = path.strip_suffix("/").unwrap();
        } else {
            target = path_parts.file_name().unwrap().to_str().unwrap();
            dir = path_parts.parent().unwrap().to_str().unwrap(); 
        };

        let doc = match &self.docker {
            Ok(d) => d,
            Err(e) => return Err(e.clone())
        };

        // Get file size
        let size = {
            let proc_size = vec![
                "sh".to_owned(), 
                "-c".to_owned(), 
                format!("tar cf - -C {} {} | wc -c", dir, target).to_owned(),
            ];
            let config = CreateExecOptions::<String> {
                attach_stdout: Some(true),
                attach_stdin: Some(false),
                cmd: Some(proc_size),
                ..Default::default()
            };
    
            let exec = doc.create_exec(container, config).await?;
            let mut out: String = "".to_string();
            if let StartExecResults::Attached { mut output, .. } = doc.start_exec(&exec.id, None).await? {
                while let Some(Ok(LogOutput::StdOut { message })) = output.next().await {
                    out.push_str(std::str::from_utf8(&message).unwrap());
                }
            } else {
                return Err(Error::DockerError("Failed to get file size in docker container".to_string()));
            }
            out.retain(|s| s.is_numeric());
            let size: u64 = FromStr::from_str(&out).expect("Failed to parse file size");
            size
        };
        // Get file stream
        let stream = {
            let proc_tar = vec![
                "tar".to_owned(), 
                "cf".to_owned(), 
                "-".to_owned(), 
                "-C".to_owned(), 
                dir.to_owned(),
                target.to_owned(),
            ];
            let config = CreateExecOptions::<String> {
                attach_stdout: Some(true),
                attach_stdin: Some(false),
                //privileged: Some(true),
                cmd: Some(proc_tar),
                ..Default::default()
            };
    
            let exec = doc.create_exec(container, config).await?;
            let (mut stdout, out) = duplex(255);
            if let StartExecResults::Attached { mut output, .. } = doc.start_exec(&exec.id, None).await? {
                tokio::spawn(async move {
                    let mut stderr = stderr();
                    while let Some(Ok(msg)) = output.next().await {
                        match msg {
                            LogOutput::StdErr { message } => {
                                match stderr.write_all(&message).await {
                                    Ok(_) => {},
                                    Err(_) => break,
                                };
                            },
                            LogOutput::StdOut { message } => {
                                match stdout.write_all(&message).await {
                                    Ok(_) => {},
                                    Err(_) => break,
                                }
                            },
                            _ => {},
                        };
                    }
                });
            } else {
                return Err(Error::DockerError("Failed to get file size in docker container".to_string()));
            }
            out
        };

        Ok(PipeCopySource::new(size, Box::new(stream)))
        
    }
   
    pub async fn get_copy_destination(&self, container: &str, path: &str) -> Result<PipeCopyDestination, Error> {
        let doc = match &self.docker {
            Ok(d) => d,
            Err(e) => return Err(e.clone())
        };
        let proc_tar = vec![
            "tar".to_owned(), 
            "xf".to_owned(), 
            "-".to_owned(), 
            "-C".to_owned(), 
            path.to_owned()
        ];
        let config = CreateExecOptions::<String> {
            attach_stdout: Some(false),
            attach_stdin: Some(true),
            attach_stderr: Some(true),
            cmd: Some(proc_tar),
            ..Default::default()
        };
        let exec = doc.create_exec(container, config).await?;
            let (mut data_stream, in_stream) = duplex(255);
            let (kill_send, kill_read) = oneshot::channel::<bool>();
            if let StartExecResults::Attached { mut input, mut output } = doc.start_exec(&exec.id, None).await? {
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
                            _ => {},
                        };
                    }
                    kill_send.send(true).ok();
                });
                tokio::spawn(async move {
                    tokio::select! {
                        _ = copy(&mut data_stream, &mut input) => {}
                        _ = kill_read => {} 
                    }
                });
            } else {
                return Err(Error::DockerError("Failed to set copy destination".to_string()));
            }
        Ok(PipeCopyDestination::new(Box::new(in_stream)))
    }


}

impl DockerEndpoint {
    pub async fn connect(&self, container_name: &str, agent: Vec<String>) -> Result<impl AsyncRead + AsyncWrite, Error> {
        let doc = match &self.get_docker() {
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

    pub async fn get_connections(&self, container_name: &str, agent: Vec<String>) -> Result<Receiver<impl AsyncRead + AsyncWrite>, Error> {
        let doc = match &self.get_docker() {
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


}