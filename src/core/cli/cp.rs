use std::fmt::Write;

use clap::{CommandFactory, ErrorKind};
use indicatif::{ProgressBar, ProgressStyle, ProgressState};

use crate::endpoint::{kube::KubeConfigs, self, PipeCopySource, PipeCopyDestination, stdio::{get_copy_destination, get_copy_source}, pipe_copy, docker::DockerEndpoint};

use super::{CopyPoint, KubeCopyPoint, Cli, DockerCopyPoint};

pub struct Cp {
    kube: KubeConfigs,
    doc: DockerEndpoint
}

enum Error {
    Docker(endpoint::docker::Error),
    Kube(endpoint::kube::Error),
    Local(endpoint::stdio::Error),
}

impl From<endpoint::docker::Error> for Error {
    fn from(e: endpoint::docker::Error) -> Self {
        Error::Docker(e)
    }
}

impl From<endpoint::kube::Error> for Error{
    fn from(e: endpoint::kube::Error) -> Self {
        Error::Kube(e)
    }
}

impl From<endpoint::stdio::Error> for Error{
    fn from(e: endpoint::stdio::Error) -> Self {
        Error::Local(e)
    }
}

impl Cp {
    pub fn new(kube: KubeConfigs, doc: DockerEndpoint) -> Self {
        Cp {kube, doc}
    }

    pub async fn exec(&self, src: CopyPoint, dst: CopyPoint) -> Result<(), Box<dyn std::error::Error>> {
        let mut cmd = Cli::command();
        print!("Copying from: ");
        let from = match get_source(&self.doc, &self.kube, src).await {
            Ok(p) => p,
            Err(_) => {
                cmd.error(ErrorKind::ArgumentConflict, "Failed to get source").exit();
            },
        };
        print!(" to: ");
        let to = match get_destination(&self.doc, &self.kube, dst).await {
            Ok(p) => p,
            Err(_) => {
                cmd.error(ErrorKind::ArgumentConflict, "Failed to get destination").exit();
            },
        };
        println!();
        let size = from.get_size();
        let mut recv = pipe_copy(from, to).await;
        tokio::spawn(async move {
            let mut total: u64 = 0;
            let pb = ProgressBar::new(size);
            pb.set_style(ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})")
                .unwrap()
                .with_key("eta", |state: &ProgressState, w: &mut dyn Write| write!(w, "{:.1}s", state.eta().as_secs_f64()).unwrap())
                .progress_chars("#>-"));
            while let Some(prog) = recv.recv().await {
                total += prog as u64;
                pb.set_position(total);
            }
            pb.finish();
            println!("Copied total of {}", human_bytes::human_bytes(total as f64));
        }).await?;
        Ok(())
    }
    
}


async fn get_source(doc: &DockerEndpoint, kube: &KubeConfigs, src: CopyPoint) -> Result<PipeCopySource, Error> {
    match src {
        CopyPoint::Kube(kp) => {
            let KubeCopyPoint{context, namespace, pod, path} = kp;
            let cp = kube.get_copy_source(context, namespace, pod, path.clone()).await?;
            print!("{}", path);
            Ok(cp)
        },
        CopyPoint::Local(path) => {
            let cp = get_copy_source(path.clone()).await?;
            print!("{}", path);
            Ok(cp)
        },
        CopyPoint::Docker(DockerCopyPoint {container, path}) => {
            let cp = doc.get_copy_source(&container, &path).await?;
            print!("{}", path);
            Ok(cp)
        },
    }

}

async fn get_destination(doc: &DockerEndpoint, kube: &KubeConfigs, dst: CopyPoint) -> Result<PipeCopyDestination, Error> {
    match dst {
        CopyPoint::Kube(kp) => {
            let KubeCopyPoint{context, namespace, pod, path} = kp;
            let cp = kube.get_copy_destination(context, namespace, pod, path.clone()).await?;
            print!("{}", path);
            Ok(cp)
        },
        CopyPoint::Docker(DockerCopyPoint {container, path}) => {
            let cp = doc.get_copy_destination(&container, &path).await?;
            print!("{}", path);
            Ok(cp)
        },
        CopyPoint::Local(path) => {
            let cp = get_copy_destination(path.clone()).await?;
            print!("{}", path);
            Ok(cp)
        },
    }

}
