//! Pane state: the last results, the row list, the run queue, and the
//! worker thread that runs jobs.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc;

use ratatui::crossterm::event::Event;
use ratatui::layout::Rect;
use ratatui::widgets::ListState;

use crate::herdr::PluginEnv;
use crate::job;
use crate::model::{Failure, RunResult};
use crate::sock::{self, Request, Response};
use crate::state::{ProjectState, Settings};
use crate::{cli, herdr};

pub use crate::job::Scope;

/// Output lines kept for the streaming panel. The full log is on disk.
const OUTPUT_LINES: usize = 500;

pub enum Msg {
    Input(Event),
    /// One line of runner output.
    Line(String),
    /// The worker finished.
    Done(Result<Vec<RunResult>, String>),
    /// A socket request; reply on the sender.
    Request(Request, mpsc::Sender<Response>),
}

/// One line in the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// Index into `App::results`.
    Build(usize),
    /// Index into `App::failures`.
    Failure(usize),
    /// The passed / skipped totals.
    Summary,
}

pub struct App {
    pub root: PathBuf,
    pub env: Option<PluginEnv>,
    pub state: ProjectState,
    pub settings: Settings,
    pub results: Vec<RunResult>,
    /// Every failure across `results`, in order.
    pub failures: Vec<Failure>,
    pub rows: Vec<Row>,
    pub list: ListState,
    pub list_area: Rect,
    pub running: Option<Scope>,
    pub queued: Option<Scope>,
    pub output: VecDeque<String>,
    /// The detail panel takes half the screen instead of a few lines.
    pub expanded: bool,
    pub status: Option<String>,
    pub quit: bool,
    pub tick: usize,
    tx: mpsc::Sender<Msg>,
}

impl App {
    pub fn new(
        root: PathBuf,
        env: Option<PluginEnv>,
        tx: mpsc::Sender<Msg>,
    ) -> Result<Self, String> {
        let state_dir = match &env {
            Some(e) => e.state_dir.clone(),
            None => std::env::temp_dir().join("herdr-testrun"),
        };
        let state = ProjectState::open(&state_dir, &root)?;
        let settings = state.settings()?;
        let mut app = Self {
            root,
            env,
            state,
            settings,
            results: Vec::new(),
            failures: Vec::new(),
            rows: Vec::new(),
            list: ListState::default(),
            list_area: Rect::default(),
            running: None,
            queued: None,
            output: VecDeque::new(),
            expanded: false,
            status: None,
            quit: false,
            tick: 0,
            tx,
        };
        if let Some(last) = app.state.last()? {
            app.set_results(last);
        }
        Ok(app)
    }

    pub fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Input(_) => {}
            Msg::Line(line) => {
                if self.output.len() == OUTPUT_LINES {
                    self.output.pop_front();
                }
                self.output.push_back(line);
            }
            Msg::Done(result) => {
                self.running = None;
                match result {
                    Ok(results) => {
                        self.set_results(results);
                        self.status = None;
                    }
                    Err(err) => self.status = Some(err),
                }
                if let Some(next) = self.queued.take() {
                    self.request(next);
                }
            }
            Msg::Request(req, reply) => {
                let response = self.serve(req);
                let _ = reply.send(response);
            }
        }
    }

    pub fn tick(&mut self) {
        if self.running.is_some() {
            self.tick = self.tick.wrapping_add(1);
        }
    }

    fn serve(&mut self, req: Request) -> Response {
        match req {
            Request::Run => Response::ok(self.request(Scope::All)),
            Request::RunFailed => match self.failed_scope() {
                Some(scope) => Response::ok(self.request(scope)),
                None => Response::err("nothing failed in the last run"),
            },
            Request::Status => Response {
                ok: true,
                message: String::new(),
                status: Some(self.status_report()),
            },
        }
    }

    pub fn status_report(&self) -> sock::Status {
        let (passed, failed, skipped) = self.totals();
        sock::Status {
            root: self.root.display().to_string(),
            running: self.running.is_some(),
            queued: self.queued.is_some(),
            passed,
            failed,
            skipped,
            build_errors: self
                .results
                .iter()
                .filter(|r| r.build_error.is_some())
                .count() as u32,
            auto_run: self.settings.auto_run,
        }
    }

    pub fn totals(&self) -> (u32, u32, u32) {
        self.results.iter().fold((0, 0, 0), |(p, f, s), r| {
            (p + r.passed, f + r.failed, s + r.skipped)
        })
    }

    pub fn total_duration(&self) -> std::time::Duration {
        self.results.iter().map(|r| r.duration).sum()
    }

    /// Runs `scope` now, or queues it behind the active run. At most one
    /// request waits; more are dropped. Returns the message to show.
    pub fn request(&mut self, scope: Scope) -> String {
        if self.running.is_some() {
            if self.queued.is_some() {
                return "a run is active and one is queued; dropped".into();
            }
            self.queued = Some(scope);
            return "queued behind the active run".into();
        }
        self.output.clear();
        self.running = Some(scope.clone());
        self.status = None;
        let root = self.root.clone();
        let env = self.env.clone();
        let state = ProjectState {
            dir: self.state.dir.clone(),
        };
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let line_tx = tx.clone();
            let result = job::run(&root, env.as_ref(), &state, &scope, |line| {
                let _ = line_tx.send(Msg::Line(line.to_string()));
            });
            let _ = tx.send(Msg::Done(result));
        });
        "running".into()
    }

    /// The failed-only scope for the last run, or `None` with nothing to
    /// rerun.
    pub fn failed_scope(&self) -> Option<Scope> {
        (!self.failures.is_empty()).then(|| Scope::Only(self.failures.clone()))
    }

    fn set_results(&mut self, results: Vec<RunResult>) {
        self.failures = results
            .iter()
            .flat_map(|r| r.failures.iter().cloned())
            .collect();
        self.rows = results
            .iter()
            .enumerate()
            .filter(|(_, r)| r.build_error.is_some())
            .map(|(i, _)| Row::Build(i))
            .chain((0..self.failures.len()).map(Row::Failure))
            .chain(std::iter::once(Row::Summary))
            .collect();
        self.results = results;
        let max = self.rows.len().saturating_sub(1);
        let sel = self.list.selected().unwrap_or(0).min(max);
        self.list.select(Some(sel));
    }

    pub fn selected_row(&self) -> Option<Row> {
        self.rows.get(self.list.selected()?).copied()
    }

    pub fn select(&mut self, index: usize) {
        if !self.rows.is_empty() {
            self.list.select(Some(index.min(self.rows.len() - 1)));
        }
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let cur = self.list.selected().unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, self.rows.len() as isize - 1);
        self.list.select(Some(next as usize));
    }

    pub fn send_to_agent(&mut self) {
        let msg = match &self.env {
            None => Err("send needs Herdr; use `herdr-testrun send --print` here".to_string()),
            Some(env) => {
                if self.failures.is_empty() {
                    Err("nothing failed in the last run".to_string())
                } else {
                    cli::send_failures(env, &self.failures)
                }
            }
        };
        self.status = Some(msg.unwrap_or_else(|e| e));
    }

    pub fn toggle_auto_run(&mut self) {
        self.settings.auto_run = !self.settings.auto_run;
        self.status = Some(match self.state.save_settings(&self.settings) {
            Ok(()) if self.settings.auto_run => {
                "auto-run on: reruns when the agent goes idle".into()
            }
            Ok(()) => "auto-run off".into(),
            Err(e) => e,
        });
    }

    /// `o`: the raw log in a Herdr popup, or its path when not under Herdr.
    pub fn open_log(&mut self) {
        let log = self.state.raw_log();
        if !log.is_file() {
            self.status = Some("no raw log yet".into());
            return;
        }
        let Some(env) = &self.env else {
            self.status = Some(format!("raw log: {}", log.display()));
            return;
        };
        let root_env = format!("HERDR_TESTRUN_ROOT={}", self.root.display());
        let result = env.run(&[
            "plugin",
            "pane",
            "open",
            "--plugin",
            &herdr::plugin_id(),
            "--entrypoint",
            "log",
            "--placement",
            "popup",
            "--env",
            &root_env,
        ]);
        self.status = result.err();
    }
}
