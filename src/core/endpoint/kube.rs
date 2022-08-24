use std::{fs::read_dir, collections::HashMap, rc::Rc, process::exit};
use home::home_dir;
use k8s_openapi::api::core::v1::{Namespace, Pod};
use kube::{config::{Kubeconfig, KubeConfigOptions, KubeconfigError}, Client, Config, api::{Portforwarder, ListParams, AttachParams}, Api, ResourceExt};
use tokio::{io::{AsyncRead, AsyncWrite, split, copy, BufReader, AsyncBufReadExt, stderr, AsyncWriteExt}, sync::mpsc::Receiver};

use super::{PipeEndpoint, AGENT, AGENT_PATH, AGENT_KILL_PATH};
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


pub struct KubeConfigs {
    contexts: HashMap<String, (Rc<KubeConfigInFile>, String)>,
    default_context: Option<String>
}
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

    pub async fn get_pod_files(&self, context: String, ns: String, pod: String, path: String) -> Result<Vec<String>, Error> {
        let client = self.get_client(context).await?;
        let pods = Api::<Pod>::namespaced(client, ns.as_str());
        let mut params = AttachParams::default();
        params.stdout = true;
        let mut proc = pods
            .exec(pod.as_str(), ["ls", "-lah", path.as_str()], &params)
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
