use crate::endpoint::{kube::KubeConfigs, docker::DockerEndpoint};

use super::path_parser::{parse_path_types, PathType, Part, K8Path};

pub struct Ls {
    kube: KubeConfigs,
    docker: DockerEndpoint
}

impl Ls {
    pub fn new(kube: KubeConfigs, docker: DockerEndpoint) -> Ls {
        Ls {kube, docker}
    }

    pub async fn get_files(&self, p: &PathType, flags: Vec<String>) -> Vec<String> {
        match p {
            PathType::Docker(part, path) => {
                if let Part::Full(container) = part {
                    return self.docker.get_container_files(container, path.as_ref().expect("Missing path"), flags).await.expect("Docker failed");
                }
                panic!("Docker container not reloved");
            },
            PathType::Kubernetes(K8Path { context, ns, pod }, path) => {
                let context: String = context.into();
                let ns: String = ns.into();
                let pod: String = pod.into();
                self.kube.get_pod_files(context, ns, pod, path.clone().expect("Missing path"), flags).await.expect("Kubernetes pod file get failed")
            },
            PathType::Fs(_) => todo!(),
        }
    }

    pub async fn exec(&self, endpoint: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
        match endpoint {
            Some(path) => {
                let p = parse_path_types(&path);
                if p.len() != 1 {
                    eprintln!("Ambiguous path!");
                }
                let res = self.get_files(&p[0], vec!["-lah".to_owned()]).await;
                for r in res {
                    println!("{}", r);
                }

                Ok(())
            },
            None => {
                for (context, file) in self.kube.get_contexts() {
                    println!("{context}({file})");
                }
                for val in self.docker.list_containers().await.expect("Failed to list docker containers") {
                    println!("{}", val);
                }
                Ok(())
            }
        }
    }
}

