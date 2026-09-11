use std::path::PathBuf;

use crate::supervisor::SupervisorClient;

#[derive(Clone)]
pub struct AppState {
    pub workspace: PathBuf,
    pub run_dir: PathBuf,
    pub conf_dir: PathBuf,
    pub supervisor: SupervisorClient,
}
