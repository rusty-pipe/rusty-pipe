use crate::endpoint::{kube::KubeConfigs, docker::DockerEndpoint};

use super::{ls::Ls, Shell};

pub struct Complete {
    kube: KubeConfigs,
    docker: DockerEndpoint
}

pub fn print_completions(shell: Shell) {
    let bin_name = "rs";
    match shell {
        Shell::Bash => {
            let comp = "complete -C 'BIN_NAME -cb' -o nospace BIN_NAME".replace("BIN_NAME", bin_name);
            println!("{comp}");
        }
    }
}

impl From<Complete> for Ls {
    fn from(c: Complete) -> Self {
        Ls::new(c.kube, c.docker)
    }
}

impl Complete {
    
    pub fn new(kube: KubeConfigs, docker: DockerEndpoint) -> Complete {
        Complete {kube, docker}
    }

    pub async fn bash_complete(self) {
        if let Ok(line) = std::env::var("COMP_LINE") {
            
            let args = line.split(" ").filter(|s| !(s.starts_with("-") && s.len() > 1)).map(|l| l.to_string()).collect::<Vec<String>>();
            // without command
            if args.len() < 3 {
                self.complete_commands(&args[1]);
                return;
            }
            
            match args[1].as_str() {
                "ls" => {
                    self.complete_ls(args[2].clone()).await;
                },
                "pf" => {
                    if args.len() > 3{
                        self.complete_pf(args[3].clone()).await;
                    } else {
                        self.complete_pf(args[2].clone()).await;
                    }
                    
                },
                _ => {}
            }
        }
        
    }
    
    fn complete_commands(&self, com: &str) {
        let commands = vec!["pf ".to_string(), "ls ".to_string()].into_iter().filter(|c| c.starts_with(com));
        print_options(&commands.collect());
        
    }
    
    async fn complete_ls(self, path: String) {
        print_options(&self.match_pod(path.as_str()).await);
        print_options(&self.match_container(path.as_str()).await);
    }
    async fn complete_pf(self, endpoint: String) {
        print_options(&self.match_local(endpoint.as_str()));
        print_options(&self.match_pod(endpoint.as_str()).await);
        print_options(&self.match_container(endpoint.as_str()).await);
    }

    fn match_local(&self, local: &str) -> Vec<String> {
        return vec![":".to_string(), "0.0.0.0:".to_string(), "- ".to_string()]
            .iter()
            .filter(|s| s.starts_with(local))
            .map(|s| s.to_owned())
            .collect::<Vec<String>>();
    }

    async fn match_pod(&self, pod: &str) -> Vec<String> {
        let contexts = self.kube.get_contexts();
        let parts = pod.split("/").map(|p| p.to_string()).collect::<Vec<String>>();

        match parts.len() {
            // <context>
            1 => {
                let res = contexts.into_iter().filter(|(c, _)| c.starts_with(parts[0].as_str()));
                // single suggestion
                if res.clone().count() == 1 {
                    return res.into_iter().map(|(c, _)| format!("{c}/")).collect();
                }
                // add description to suggestions
                return res.into_iter().map(|(c, f)| format!("{c} --{f}")).collect();
            },
            // <context>/
            2 => {
                let mut res = vec![];
                for val in self.kube.get_ns(parts[0].clone()).await.ok().unwrap_or_else(|| vec![]) {
                    if val.starts_with(parts[1].as_str()) {
                        res.push(format!("{}/{}/", parts[0], val));
                    }
                }
                return res;
            },
            // <context>/<ns>/
            3 => {
                let mut res = vec![];
                for val in self.kube.get_pods(parts[0].clone(), parts[1].clone()).await.ok().unwrap_or_else(|| vec![]) {
                    if val.starts_with(parts[2].as_str()) {
                        res.push(format!("{}/{}/{}:", parts[0], parts[1], val));
                    }
                }
                return res;
            },
            _ => {
                return vec![];
            }
        };

    }

    async fn match_container(&self, container: &str) -> Vec<String>  {
        if container.contains(":") {
            return vec![];
        }
        let containers = self.docker
            .list_containers().await.ok()
            .unwrap_or_else(|| vec![])
            .into_iter()
            .map(|c| format!("{c}:")).collect();
        if container.is_empty() {
            return containers;
        }
        return containers.into_iter().filter(|c| c.starts_with(container)).collect(); 
    }

    


}

fn print_options(options: &Vec<String>) {
    for o in options {
        println!("{o}");
    }
}