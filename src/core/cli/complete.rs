use crate::endpoint::{kube::KubeConfigs, docker::DockerEndpoint};

use super::{ls::Ls, Shell, path_parser::get_suggestions};

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
                "cp" => {
                    if args.len() > 3{
                        self.complete_cp(args[3].clone()).await;
                    } else {
                        self.complete_cp(args[2].clone()).await;
                    }
                    
                },
                _ => {}
            }
        }
        
    }
    
    fn complete_commands(&self, com: &str) {
        let commands = vec!["pf ".to_string(), "ls ".to_string(), "cp ".to_string()].into_iter().filter(|c| c.starts_with(com));
        print_options(&commands.collect());
        
    }

    async fn complete_cp(self, path: String) {
        print_options(&get_suggestions(Some(path), &self.docker, &self.kube, true).await);
    }
    
    async fn complete_ls(self, path: String) {
        print_options(&get_suggestions(Some(path), &self.docker, &self.kube, true).await);
    }
    async fn complete_pf(self, endpoint: String) {
        print_options(&self.match_local(endpoint.as_str()));
        if !endpoint.contains(":") {
            print_options(&get_suggestions(Some(endpoint), &self.docker, &self.kube, false).await);
        }
    }

    fn match_local(&self, local: &str) -> Vec<String> {
        return vec![":".to_string(), "0.0.0.0:".to_string(), "- ".to_string()]
            .iter()
            .filter(|s| s.starts_with(local))
            .map(|s| s.to_owned())
            .collect::<Vec<String>>();
    }

}

fn print_options(options: &Vec<String>) {
    for o in options {
        println!("{o}");
    }
}