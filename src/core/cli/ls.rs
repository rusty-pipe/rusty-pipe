use crate::endpoint::{kube::KubeConfigs, docker::DockerEndpoint};

pub struct Ls {
    kube: KubeConfigs,
    docker: DockerEndpoint
}

impl Ls {
    pub fn new(kube: KubeConfigs, docker: DockerEndpoint) -> Ls {
        Ls {kube, docker}
    }

    pub async fn exec(&self, endpoint: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
        match endpoint {
            Some(path) => {
                let url_and_path: Vec<String> = path.split(":").map(|p| {String::from(p)}).collect();
                let parts: Vec<String> = url_and_path[0].split("/").map(|p| {String::from(p)}).collect();
                match parts.len() {
                    0 => {
                        
                        for val in self.kube.get_contexts() {
                            println!("{}", val.0);
                        }
                        
                    }
                    1 => {
                        // <container>:/
                        if !url_and_path[1].is_empty() {
                            for val in self.docker.get_container_files(&parts[0], &url_and_path[1]).await.unwrap() 
                            {
                                println!("{val}");
                            }
                        }
                        return Ok(());
                    },
                    // <context>/
                    2 => {
                        if !parts[1].is_empty() {
                            return Ok(());
                        }
                        for val in self.kube.get_ns(parts[0].clone()).await.expect("Bad context") {
                            println!("{}/{}", parts[0], val);
                        }
                    },
                    3 => {
                        // <context>/<ns>/<pod>:/
                        if !parts[2].is_empty() {
                            if !url_and_path[1].is_empty() {
                                for val in self.kube.get_pod_files(
                                    parts[0].clone(), 
                                    parts[1].clone(), 
                                    parts[2].clone(), 
                                    url_and_path[1].clone()
                                ).await.unwrap() 
                                {
                                    println!("{val}");
                                }
                            }
                            return Ok(());
                        }
                        // <context>/<ns>/
                        for val in self.kube.get_pods(parts[0].clone(), parts[1].clone()).await.expect("Bad context or namespace") {
                            println!("{}/{}/{}", parts[0], parts[1], val);
                        }
                    },
                    _ => {}
                };
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