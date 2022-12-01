use std::{fs::read_dir, collections::HashMap, rc::Rc, process::exit, str::FromStr, path::Path};
use home::home_dir;
use k8s_openapi::api::core::v1::{Namespace, Pod};
use kube::{config::{Kubeconfig, KubeConfigOptions, KubeconfigError}, Client, Config, api::{Portforwarder, ListParams, AttachParams}, Api, ResourceExt};
use tokio::{io::{AsyncRead, AsyncWrite, split, copy, BufReader, AsyncBufReadExt, stderr, AsyncWriteExt, AsyncReadExt}, sync::mpsc::Receiver, select, time::sleep};

use super::{PipeEndpoint, AGENT, AGENT_PATH, AGENT_KILL_PATH, PipeCopySource, PipeCopyDestination};
use crate::mux::Multiplexer;

// Files holding Kube config
struct KubeConfigInFile {
    config: Kubeconfig,
    file: String
}

#[derive(Debug)]
pub enum Error {
    KubeError(kube::Error),
    KubeConfigError(KubeconfigError),
    ContextNotFound(String),
    ExecError(String)
}

impl From<kube::Error> for Error {
    fn from(e: kube::Error) -> Self {
        Self::KubeError(e)
    }
}

impl From<KubeconfigError> for Error {
    fn from(e: KubeconfigError) -> Self {
        Self::KubeConfigError(e)
    }
}

#[cfg_attr(test, faux::create)]
pub struct KubeConfigs {
    contexts: HashMap<String, (Rc<KubeConfigInFile>, String)>,
    default_context: Option<String>
}
#[cfg_attr(test, faux::methods)]
impl KubeConfigs {
    pub fn new() -> Self {
        let kube_dir = format!("{}/.kube", home_dir().unwrap().display());
        let mut contexts = HashMap::new();
        let mut default_context = None;
        let paths = read_dir(kube_dir).unwrap();
        let mut unique_paths = vec![]; 
        
        for file in paths {
            let dir = file.unwrap().path();
            let path = dir.to_str().unwrap().to_string();
            let unique_path = std::fs::canonicalize(dir.clone()).unwrap().to_str().unwrap().to_string();
            // skip same paths that symlink is pointing
            if unique_paths.contains(&unique_path) {
                continue;
            }
            unique_paths.push(unique_path);

            if path.ends_with(".config") || path.ends_with("/config") {
                let kube_config = Rc::new(KubeConfigInFile {
                    config: Kubeconfig::read_from(path.clone()).unwrap(), 
                    file: path.clone()
                });
                default_context = kube_config.config.current_context.clone();

                for context in &kube_config.config.contexts {
                    let mut context_name = context.name.clone();
                    if contexts.contains_key(&context_name) {
                       let mut i = 2;
                       while contexts.contains_key(&format!("{context_name}_{i}")) {
                           i += 1;
                       }
                       context_name = format!("{context_name}_{i}");
                    }
                    contexts.insert(context_name.clone(), (kube_config.clone(), context.name.clone()));
                    log::info!("Found context: {} in {}", context_name, path.clone());
                }
            }
        }
        KubeConfigs { contexts: contexts, default_context: default_context }
    }
    
    // -> (context, file)
    pub fn get_contexts(&self) -> Vec<(String, String)> {
        self.contexts.keys().map(|key|{
            let (file, _) =  self.contexts.get(key).unwrap();
            (key.clone(), file.file.clone())
        }).collect()
    }

    pub async fn get_ns(&self, context: String) -> Result<Vec<String>, Error> {
        let client = self.get_client(context).await?;
        let ns = Api::<Namespace>::all(client);
        let list = ns.list(&ListParams::default()).await?;
        Ok(list.into_iter().map(|n| {n.name_unchecked()}).collect())
    }

    pub async fn get_pods(&self, context: String, ns: String) -> Result<Vec<String>, Error> {
        let client = self.get_client(context).await?;
        let pods = Api::<Pod>::namespaced(client, ns.as_str());
        let list = pods.list(&ListParams::default()).await?;
        return Ok(list.into_iter().map(|n| {n.name_unchecked()}).collect());
    }

    pub async fn get_pod_files(&self, context: String, ns: String, pod: String, path: String, mut flags: Vec<String>) -> Result<Vec<String>, Error> {
        let client = self.get_client(context).await?;
        let pods = Api::<Pod>::namespaced(client, ns.as_str());
        let mut params = AttachParams::default();
        params.stdout = true;
        let mut cmd = vec!["ls".to_owned(), path];
        cmd.append(&mut flags);
        let mut proc = pods
            .exec(pod.as_str(), cmd, &params)
            .await?;
        let stdout = proc.stdout().ok_or(Error::ExecError("Failed to exec ls".to_string()))?;
        let mut lines = BufReader::new(stdout).lines();
        let mut lines_vec = vec![];
        while let Some(line) = lines.next_line().await.unwrap() {
            lines_vec.push(line);
        }

        return Ok(lines_vec);
    }

    pub async fn install_agent(&self, context: String, ns: String, pod: String) -> Result<(), Error> {
        let client = self.get_client(context).await?;
        let pods = Api::<Pod>::namespaced(client, ns.as_str());
        // upload file
        let mut params = AttachParams::default();
        params.stdin = true;
        let mut upload_proc = pods
            .exec(pod.as_str(), ["dd", format!("of={}", AGENT_PATH).as_str()], &params)
            .await?;
        let mut stdin = upload_proc.stdin().ok_or(Error::ExecError("Failed to exec dd".to_string()))?;
        stdin.write_all(AGENT).await.or(Err(Error::ExecError("Failed to exec tee".to_string())))?;
        stdin.shutdown().await.expect("Shutdown of agent copy failed");
        drop(stdin);
        // chmod
        params.stdin = false;
        params.stdout = false;
        let chmod_proc = pods
            .exec(pod.as_str(), ["chmod", "+x", AGENT_PATH], &params)
            .await?;
        chmod_proc.join().await.or(Err(Error::ExecError("Failed to exec chmod".to_string())))?;
        return Ok(());
    }


    pub async fn get_default_client(&self) -> Result<Client, Error> {
        if let Some(context) = self.default_context.clone() {
            return self.get_client(context).await;
        } else {
            return Ok(Client::try_default().await?);
        }
    }

    pub async fn get_client(&self, context: String) -> Result<Client, Error> {
        let (config, original_context) = self.contexts.get(&context).ok_or(Error::ContextNotFound(context.clone()))?;
        let options: KubeConfigOptions = KubeConfigOptions{ context: Some(original_context.clone()), cluster: None, user: None};
        let client_config = Config::from_custom_kubeconfig(
            config.config.clone(),
            &options
        ).await?;

        return Ok(Client::try_from(client_config)?);
    }

    pub async fn get_port_forward(&self, context: String, ns: String, pod: String, port: u16) ->  Result<PortForwardPipeEndpoint, Error> {
        let client = self.get_client(context).await?;
        let pods = Api::<Pod>::namespaced(client, &ns);
        let pf = pods.portforward(&pod, &[port]).await?;
        Ok(PortForwardPipeEndpoint::new(pf, port))
    }

    pub async fn get_copy_source(&self, context: String, ns: String, pod: String, path: String) ->  Result<PipeCopySource, Error> {
        let client = self.get_client(context).await?;
        let pods = Api::<Pod>::namespaced(client, &ns);
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
        let mut params = AttachParams::default();
        params.stderr = true;
        params.stdin = false;
        params.stdout = true;
        let mut proc_size = pods.exec(&pod, [
            "sh".to_owned(), 
            "-c".to_owned(), 
            format!("tar cf - -C {} {} | wc -c", dir, target).to_owned(), 
        ], &params).await?;
        
        let mut stderr_stream = proc_size.stderr().expect("Remote stderr failed");
        let mut stdout_stream = proc_size.stdout().expect("Remote stdout failed");
        let mut stdout_buf = vec![];
        let mut stderr_buf = vec![];

        stderr_stream.read_to_end(&mut stderr_buf).await.or(Err(Error::ExecError("Failed to exec wc".to_owned())))?;
        let e = std::str::from_utf8(&mut stderr_buf).expect("Failed to parse return");
        if !e.is_empty() {
            return Err(Error::ExecError(e.to_owned()));
        }

        stdout_stream.read_to_end(&mut stdout_buf).await.or(Err(Error::ExecError("Failed to exec wc".to_owned())))?;
        let mut du_res = String::from_utf8_lossy(&stdout_buf).to_string();
        du_res.retain(|s| s.is_numeric());
        let size: u64 = FromStr::from_str(du_res.as_str()).expect("Failed to parse file size");
        let mut proc_tar = pods.exec(&pod, [
            "tar".to_owned(), 
            "cf".to_owned(), 
            "-".to_owned(), 
            "-C".to_owned(), 
            dir.to_owned(),
            target.to_owned(),
        ], &params).await?;

        let mut stderr_stream = proc_tar.stderr().expect("Remote stderr failed");
        let stdout_stream = proc_tar.stdout().expect("Remote stdout failed");
        
        // wait for a sec just to see if there is some err
        select! {
            _ = sleep(tokio::time::Duration::from_millis(1000)) => {
                
            },
            _ = stderr_stream.read_to_end(&mut stderr_buf) => {
                let e = std::str::from_utf8(&mut stderr_buf).expect("Failed to parse return");
                return Err(Error::ExecError(e.to_owned()));
            }
        };
        // prevent process from being dropped
        tokio::spawn(proc_tar.join());

        Ok(PipeCopySource::new(size, Box::new(stdout_stream)))
    }

    pub async fn get_copy_destination(&self, context: String, ns: String, pod: String, path: String) -> Result<PipeCopyDestination, Error> {
        let client = self.get_client(context).await?;
        let pods = Api::<Pod>::namespaced(client, &ns);
        let mut params = AttachParams::default();
        let mut stderr_buf = vec![];
        params.stderr = true;
        params.stdin = true;
        params.stdout = false;

        let mut proc_tar = pods.exec(&pod, [
            "tar".to_owned(), 
            "xf".to_owned(), 
            "-".to_owned(), 
            "-C".to_owned(), 
            path.to_owned()
        ], &params).await?;

        let mut stderr_stream = proc_tar.stderr().expect("Remote stderr failed");
        let stdin_stream = proc_tar.stdin().expect("Remote stdin failed");
        
        // wait for a sec just to see if there is some err
        select! {
            _ = sleep(tokio::time::Duration::from_millis(1000)) => {
                
            },
            _ = stderr_stream.read_to_end(&mut stderr_buf) => {
                let e = std::str::from_utf8(&mut stderr_buf).expect("Failed to parse return");
                return Err(Error::ExecError(e.to_owned()));
            }
        };
        // prevent process from being dropped
        tokio::spawn(proc_tar.join());
        
        Ok(PipeCopyDestination::new(Box::new(stdin_stream)))
    }

    
}

impl KubeConfigs {
    pub async fn get_connections(&self, context: String, ns: String, pod: String, agent: &[String]) -> Result<Receiver<impl AsyncRead + AsyncWrite + Unpin + Send + 'static>, Error> {
        let client = self.get_client(context).await?;
        let pods = Api::<Pod>::namespaced(client, &ns);
        let mut params = AttachParams::default();
        params.stderr = true;
        params.stdin = true;
        params.stdout = true;
        let mut proc = pods.exec(&pod, agent, &params).await?;
        
        let mut stderr_stream = proc.stderr().expect("Remote stderr failed");
        tokio::spawn(async move {
            copy(&mut stderr_stream, &mut stderr()).await
        });
        let cons = Multiplexer::new().produce_connections(proc.stdout().expect("Remote stdout failed"), proc.stdin().expect("Remote stdin failed"));
        // kill switch of remote agent
        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.unwrap();
            params.stderr = false;
            params.stdin = false;
            params.stdout = true;
            let kill_proc = pods.exec(&pod, ["touch", AGENT_KILL_PATH], &params).await.unwrap();
            kill_proc.join().await.unwrap();
            exit(0);
        });

        Ok(cons)
    }
}


pub struct PortForwardPipeEndpoint {
    pf: Portforwarder,
    port: u16
}

impl PortForwardPipeEndpoint {
    pub fn new(pf: Portforwarder, port: u16) -> Self {
        return PortForwardPipeEndpoint{pf: pf, port: port};
    }
}

impl PipeEndpoint for PortForwardPipeEndpoint {
    fn get_sink_and_source(mut self: Box<Self>) -> (Box<(dyn tokio::io::AsyncRead + Unpin + std::marker::Send + 'static)>, Box<(dyn tokio::io::AsyncWrite + Unpin + std::marker::Send + 'static)>){
        let stream = self.pf.take_stream(self.port).unwrap();
        let (stream, sink) = split(stream);
        return (Box::new(stream), Box::new(sink));
    }
}
