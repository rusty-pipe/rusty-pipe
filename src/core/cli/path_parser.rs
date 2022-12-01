use crate::endpoint::{docker::DockerEndpoint, kube::KubeConfigs};



pub struct K8Path {
    pub context: Part,
    pub ns: Part,
    pub pod: Part
}


pub enum Part {
    Full(String),
    Partial(String)
}

impl From<Part> for String {
    fn from(p: Part) -> Self {
        match p {
            Part::Full(s) => s,
            Part::Partial(s) => s,
        }
    }
}

impl From<&Part> for String {
    fn from(p: &Part) -> Self {
        match p {
            Part::Full(s) => s.to_owned(),
            Part::Partial(s) => s.to_owned(),
        }
    }
}

pub enum PathType {
    Kubernetes(K8Path, Option<String>),
    Docker(Part, Option<String>),
    Fs(String)
}

impl PathType {
    pub async fn get_docker_suggestions(&self, docker: &DockerEndpoint, resolve_fs: bool) -> Vec<String> {
        if let PathType::Docker(container, path) = self {
            match container {
                Part::Partial(container_name) => {
                    let arr: Vec<_> = docker.list_containers().await
                        .unwrap_or(vec![])
                        .iter()
                        .filter_map(|c| {
                            if c.starts_with(container_name) {
                                Some(format!("{}:", c))
                            } else {
                                None
                            }
                        })
                        .collect();
                    return arr;
                },
                Part::Full(container_name) => {
                    if !resolve_fs {
                        return vec![format!("{}:", container_name)];
                    }
                    let path = path.clone().unwrap_or("".to_owned());
                    // /path/ | /
                    if path.ends_with("/") {
                        if let Ok(arr) = docker.get_container_files(container_name, &path, vec![]).await {
                            let maped: Vec<_> = arr.iter().map(|f| format!("{}{}", path, f)).collect();
                            return maped;
                        }
                    }
                    // /path/file
                    if path.len() > 1 {
                        let parts: Vec<_> = path.split("/").map(|p| p.to_owned()).collect();
                        if let Some((file, parts)) = parts.split_last() {
                            let path = parts.join("/");
                            if let Ok(arr) = docker.get_container_files(container_name, &path, vec![]).await {
                                let maped: Vec<_> = arr.iter().filter_map(|f| {
                                    if f.starts_with(file) {
                                        Some(format!("{}/{}", path, f))
                                    } else {
                                        None
                                    }
                                }).collect();
                                return maped;
                            }
                        }
                    }
    
                },
            };
        }
        vec![]
    }

    pub async fn get_k8_suggestions(&self, kube: &KubeConfigs, resolve_fs: bool) -> Vec<String> {
        if let PathType::Kubernetes(k8_path, path) = self {
            if let Part::Partial(context) = &k8_path.context {
                let arr: Vec<_> = kube
                    .get_contexts()
                    .iter()
                    .filter_map(|(con,_)| {
                        if con.starts_with(context) {
                            Some(format!("{}/", con))
                        } else {
                            None
                        }
                    })
                    .collect();
                return arr;
            }
    
            let context: String = (&k8_path.context).into();
    
            if let Part::Partial(ns) = &k8_path.ns {
                let arr: Vec<_> = kube
                    .get_ns(context.clone()).await
                    .unwrap_or(vec![])
                    .iter()
                    .filter_map(|n| {
                        if n.starts_with(ns) {
                            Some(format!("{}/{}/", context, n))
                        } else {
                            None
                        }
                    })
                    .collect();
                return arr;
            }
    
            let ns: String = (&k8_path.ns).into();
    
            if let Part::Partial(pod) = &k8_path.pod {
                let arr: Vec<_> = kube
                    .get_pods(context.clone(), ns.clone()).await
                    .unwrap_or(vec![])
                    .iter()
                    .filter_map(|p| {
                        if p.starts_with(pod) {
                            Some(format!("{}/{}/{}:", context, ns, p))
                        } else {
                            None
                        }
                    })
                    .collect();
                return arr;
            }
    
            let pod: String = (&k8_path.pod).into();

            if !resolve_fs {
                return vec![format!("{}/{}/{}:", context, ns, pod)];
            }
    
            let path = path.clone().unwrap_or("".to_owned());
            // /path/ | /
            if path.ends_with("/") {
                if let Ok(arr) = kube.get_pod_files(context.clone(), ns.clone(), pod.clone(), path.clone(), vec![]).await {
                    let maped: Vec<_> = arr.iter().map(|f| {
                        format!("{}{}", path, f)
                    }).collect();
                    return maped;
                }
            }
            // /path/file
            if path.len() > 1 {
                let parts: Vec<_> = path.split("/").map(|p| p.to_owned()).collect();
                if let Some((file, p)) = parts.split_last() {
                    let mut path = p.join("/");
                    if path.len() < 1 {
                        path = "/".to_owned();
                    } else {
                        path = format!("{}/", path);
                    }
                    if let Ok(arr) = kube.get_pod_files(context.clone(), ns.clone(), pod.clone(), path.clone(), vec![]).await {
                        let maped: Vec<_> = arr.iter().filter_map(|f| {
                            if f.starts_with(file) {
                                //Some(format!("{}/{}/{}:{}/{}", context, ns, pod, path, f))
                                Some(format!("{}{}", path, f))
                            } else {
                                None
                            }
                        }).collect();
                        return maped;
                    }
                }
            }
        }
        vec![]
    }

}

pub fn parse_docker_and_k8_partials(path: &str) -> Vec<PathType> {
    let mut res = vec![];
    // container | context
    if !path.contains("/") {
        res.push(PathType::Docker(Part::Partial(path.to_owned()), None));
        res.push(PathType::Kubernetes(K8Path { 
            context: Part::Partial(path.to_owned()), 
            ns: Part::Partial("".to_owned()),
            pod: Part::Partial("".to_owned()), 
        }, None));
        return res;
    } 
    // context
    else {
        let parts: Vec<String> = path.split("/").map(|p| {String::from(p)}).collect();
        match parts.len() {
            2 => {
                let kube = K8Path {
                    context: Part::Full(parts[0].to_owned()),
                    ns: Part::Partial(parts[1].to_owned()),
                    pod: Part::Partial("".to_owned())
                };
                res.push(PathType::Kubernetes(kube, None));
                return res;
            },
            3 => {
                let kube = K8Path {
                    context: Part::Full(parts[0].to_owned()),
                    ns: Part::Full(parts[1].to_owned()),
                    pod: Part::Partial(parts[2].to_owned())
                };
                res.push(PathType::Kubernetes(kube, None));
                return res;
            },
            _ => return res
        }
    }
}

pub fn parse_path_types(path: &str) -> Vec<PathType> {
    let mut res = vec![];

    if path.starts_with(".") || path.starts_with("/") || path.starts_with("~") || path.starts_with("..") {
        res.push(PathType::Fs(path.to_owned()));
        return res;
    }
    
    // partials
    if !path.contains(":") {
        return parse_docker_and_k8_partials(path);
    }

    let url_and_path: Vec<String> = path.split(":").map(|p| {String::from(p)}).collect();
    let parts: Vec<String> = url_and_path[0].split("/").map(|p| {String::from(p)}).collect();
    match parts.len() {
        // <container>:/
        1 => {
            res.push(PathType::Docker(Part::Full(url_and_path[0].to_owned()), Some(url_and_path[1].to_owned())));
        },
        // <context>/<ns>/<pod>:/
        3 => {
            let kube = K8Path {
                context: Part::Full(parts[0].to_owned()),
                ns: Part::Full(parts[1].to_owned()),
                pod: Part::Full(parts[2].to_owned())
            };
            res.push(PathType::Kubernetes(kube, Some(url_and_path[1].to_owned())));
        },
        _ => {}
    };

    res
}


pub async fn get_suggestions(endpoint: Option<String>, docker: &DockerEndpoint, kube: &KubeConfigs, resolve_fs: bool) -> Vec<String> {
    match endpoint {
        Some(e) => {
            let mut res = vec![];
            let parsed = parse_path_types(&e);
            for p in parsed {
                match &p {
                    PathType::Docker(_, _) => {
                        let mut sug = p.get_docker_suggestions(docker, resolve_fs).await;
                        res.append(&mut sug);
                    },
                    PathType::Kubernetes(_, _) => {
                        let mut sug = p.get_k8_suggestions(kube, resolve_fs).await;
                        res.append(&mut sug);
                    },
                    PathType::Fs(_) => todo!(),
                }
            }
            res
        },
        None => {
            let mut res = vec![];
            for (context, _) in kube.get_contexts() {
                res.push(context);
            }
            for val in docker.list_containers().await.expect("Failed to list docker containers") {
                res.push(val);
            }
            res
        }
    }
}


#[cfg(test)]
mod tests {
    use crate::{endpoint::{kube::KubeConfigs, docker::DockerEndpoint}, cli::path_parser::get_suggestions};

    fn get_docker_and_k8() -> (DockerEndpoint, KubeConfigs) {
        let mut kube = KubeConfigs::faux();
        let mut docker = DockerEndpoint::faux();
        faux::when!(kube.get_contexts).then(|_| {vec![
            ("k8con1".to_owned(), "".to_owned()),
            ("k8con11".to_owned(), "".to_owned()),
            ("k8con2".to_owned(), "".to_owned())
        ]});

        faux::when!(kube.get_ns).then(|_|{Ok(vec![
            "ns1".to_owned(),
            "ns11".to_owned(),
            "ns2".to_owned(),
        ])});

        faux::when!(kube.get_pods).then(|_|{Ok(vec![
            "pod1".to_owned(),
            "pod11".to_owned(),
            "pod2".to_owned(),
        ])});

        faux::when!(kube.get_pod_files).then(|_|{Ok(vec![
            "file1".to_owned(),
            "file11".to_owned(),
            "file2".to_owned(),
        ])});

        faux::when!(docker.list_containers).then(|_|{Ok(vec![
            "con1".to_owned(),
            "con11".to_owned(),
            "con2".to_owned(),
        ])});

        faux::when!(docker.get_container_files).then(|_|{Ok(vec![
            "file1".to_owned(),
            "file11".to_owned(),
            "file2".to_owned(),
        ])});

        (docker, kube)
    }

    // kubernetes
    #[tokio::test]
    async fn k8_context_completion_works() {
        let (docker, kube) = get_docker_and_k8();
        let res = get_suggestions(Some("".to_owned()), &docker, &kube, true).await;
        assert!(res.contains(&"k8con1/".to_owned()));
        assert!(res.contains(&"k8con11/".to_owned()));
        assert!(res.contains(&"k8con2/".to_owned()));

        let res = get_suggestions(Some("k8con1".to_owned()), &docker, &kube, true).await;
        assert!(res.contains(&"k8con1/".to_owned()));
        assert!(res.contains(&"k8con11/".to_owned()));

        let res = get_suggestions(Some("k8con2".to_owned()), &docker, &kube, true).await;
        assert!(res.contains(&"k8con2/".to_owned()));
        
    }

    #[tokio::test]
    async fn k8_ns_completion_works() {
        let (docker, kube) = get_docker_and_k8();
        let res = get_suggestions(Some("k8con1/".to_owned()), &docker, &kube, true).await;
        assert!(res.contains(&"k8con1/ns1/".to_owned()));
        assert!(res.contains(&"k8con1/ns11/".to_owned()));
        assert!(res.contains(&"k8con1/ns2/".to_owned()));

        let res = get_suggestions(Some("k8con1/ns1".to_owned()), &docker, &kube, true).await;
        assert!(res.contains(&"k8con1/ns1/".to_owned()));
        assert!(res.contains(&"k8con1/ns11/".to_owned()));

        let res = get_suggestions(Some("k8con1/ns2".to_owned()), &docker, &kube, true).await;
        assert!(res.contains(&"k8con1/ns2/".to_owned()));
        
    }

    #[tokio::test]
    async fn k8_pod_completion_works() {
        let (docker, kube) = get_docker_and_k8();
        let res = get_suggestions(Some("k8con1/ns1/".to_owned()), &docker, &kube, true).await;
        assert!(res.contains(&"k8con1/ns1/pod1:".to_owned()));
        assert!(res.contains(&"k8con1/ns1/pod11:".to_owned()));
        assert!(res.contains(&"k8con1/ns1/pod2:".to_owned()));

        let res = get_suggestions(Some("k8con1/ns1/pod1".to_owned()), &docker, &kube, true).await;
        assert!(res.contains(&"k8con1/ns1/pod1:".to_owned()));
        assert!(res.contains(&"k8con1/ns1/pod11:".to_owned()));

        let res = get_suggestions(Some("k8con1/ns1/pod2".to_owned()), &docker, &kube, true).await;
        assert!(res.contains(&"k8con1/ns1/pod2:".to_owned()));
        
    }

    #[tokio::test]
    async fn k8_file_completion_works() {
        let (docker, kube) = get_docker_and_k8();
        let res = get_suggestions(Some("k8con1/ns1/pod1:/".to_owned()), &docker, &kube, true).await;
        assert!(res.contains(&"/file1".to_owned()));
        assert!(res.contains(&"/file11".to_owned()));
        assert!(res.contains(&"/file2".to_owned()));

        let res = get_suggestions(Some("k8con1/ns1/pod1:/file1".to_owned()), &docker, &kube, true).await;
        assert!(res.contains(&"/file1".to_owned()));
        assert!(res.contains(&"/file11".to_owned()));

        let res = get_suggestions(Some("k8con1/ns1/pod1:/file2".to_owned()), &docker, &kube, true).await;
        assert!(res.contains(&"/file2".to_owned()));

        let res = get_suggestions(Some("k8con1/ns1/pod1:/file1/".to_owned()), &docker, &kube, true).await;
        assert!(res.contains(&"/file1/file1".to_owned()));
        assert!(res.contains(&"/file1/file11".to_owned()));
        assert!(res.contains(&"/file1/file2".to_owned()));
        
    }

    // docker
    #[tokio::test]
    async fn container_completion_works() {
        let (docker, kube) = get_docker_and_k8();
        let res = get_suggestions(Some("".to_owned()), &docker, &kube, true).await;
        assert!(res.contains(&"con1:".to_owned()));
        assert!(res.contains(&"con11:".to_owned()));
        assert!(res.contains(&"con2:".to_owned()));

        let res = get_suggestions(Some("con1".to_owned()), &docker, &kube, true).await;
        assert!(res.contains(&"con1:".to_owned()));
        assert!(res.contains(&"con11:".to_owned()));

        let res = get_suggestions(Some("con2".to_owned()), &docker, &kube, true).await;
        assert!(res.contains(&"con2:".to_owned()));
        
    }

    #[tokio::test]
    async fn container_file_completion_works() {
        let (docker, kube) = get_docker_and_k8();
        let res = get_suggestions(Some("con1:/".to_owned()), &docker, &kube, true).await;
        assert!(res.contains(&"/file1".to_owned()));
        assert!(res.contains(&"/file11".to_owned()));
        assert!(res.contains(&"/file2".to_owned()));

        let res = get_suggestions(Some("con1:/file1".to_owned()), &docker, &kube, true).await;
        assert!(res.contains(&"/file1".to_owned()));
        assert!(res.contains(&"/file11".to_owned()));

        let res = get_suggestions(Some("con1:/file2".to_owned()), &docker, &kube, true).await;
        assert!(res.contains(&"/file2".to_owned()));

        let res = get_suggestions(Some("con1:/file1/".to_owned()), &docker, &kube, true).await;
        assert!(res.contains(&"/file1/file1".to_owned()));
        assert!(res.contains(&"/file1/file11".to_owned()));
        assert!(res.contains(&"/file1/file2".to_owned()));
        
    }
   

}
