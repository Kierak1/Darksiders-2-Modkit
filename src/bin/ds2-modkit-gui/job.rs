use std::sync::mpsc;
use std::time::Instant;

use crate::app::App;

pub(crate) enum Action {
    None,
    ApplyMod(usize),
    ExtractAsset(usize),
    ReplaceAsset(usize),
    RefreshAll,
    RefreshAssets,
    RestoreScripts,
    RestorePaks,
    BrowseMedia,
}

pub(crate) enum Msg {
    Log(String),
    Progress {
        label: String,
        done: u64,
        total: u64,
    },
    /// The background item DB finished building.
    SaveDb(Box<crate::save::SaveDb>),
    Done,
}

pub(crate) struct Job {
    pub(crate) label: String,
    pub(crate) rx: mpsc::Receiver<Msg>,
    pub(crate) started: Instant,
    pub(crate) progress: Option<(String, u64, u64)>,
    pub(crate) refresh: bool,
}

impl App {
    pub(crate) fn start_job(
        &mut self,
        label: String,
        refresh: bool,
        run: impl FnOnce(&mpsc::Sender<Msg>) + Send + 'static,
    ) {
        if self.job.is_some() {
            self.log("busy: a job is already running");
            return;
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            run(&tx);
            let _ = tx.send(Msg::Done);
        });
        self.log(format!("{label} ..."));
        self.job = Some(Job {
            label,
            rx,
            started: Instant::now(),
            progress: None,
            refresh,
        });
    }

    pub(crate) fn poll_job(&mut self) {
        let mut logs = Vec::new();
        let mut done = false;
        if let Some(job) = &mut self.job {
            loop {
                match job.rx.try_recv() {
                    Ok(Msg::Log(line)) => logs.push(line),
                    Ok(Msg::Progress { label, done, total }) => {
                        job.progress = Some((label, done, total));
                    }
                    Ok(Msg::SaveDb(db)) => {
                        self.save_db = Some(*db);
                    }
                    Ok(Msg::Done) => {
                        done = true;
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        done = true;
                        break;
                    }
                }
            }
        }
        for line in logs {
            self.log(line);
        }
        if done {
            if let Some(job) = self.job.take() {
                self.log(format!(
                    "{} finished in {}s",
                    job.label,
                    job.started.elapsed().as_secs()
                ));
                if job.refresh {
                    self.refresh_assets();
                }
            }
        }
    }
}
