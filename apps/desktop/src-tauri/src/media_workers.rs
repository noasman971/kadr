//! The threads that own decoders: one for preview frames, one for thumbnails.
//!
//! Video decoders cannot move between threads (see `CLAUDE.md`), so each kind of
//! picture work has a thread of its own, created once, that keeps its decoders
//! open between requests. Commands send a job and wait for the answer on a
//! channel, from a blocking task so the interface's thread never waits.
//!
//! The preview thread only ever renders the **newest** waiting job: while the
//! user drags the playhead, older positions are answered "superseded" instead of
//! piling up.
//!
//! Both read the proxies of heavy videos when they are ready (decision record
//! 0055), listed in [`ReadyProxies`]. Both can be asked to close every file
//! they hold, so the proxy cache can be cleared.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use kadr_core::media_asset::PictureSize;
use kadr_core::project::Project;
use kadr_core::time::Timestamp;
use kadr_engine::RendererPreference;
use kadr_engine::media_overview::ThumbnailMaker;
use kadr_engine::preview::PreviewRenderer;
use kadr_engine::proxies::ReadyProxies;
use kadr_media::MediaBackend;

use crate::binary_messages::{encode_rgba_picture, encode_yuv_picture};
use crate::command_error::{CommandError, CommandResult, ErrorCode};

/// The answer to a picture job: the encoded picture.
pub type PictureReply = CommandResult<Vec<u8>>;

/// A request for a preview frame.
pub struct PreviewJob {
    /// The project as it was when the frame was asked for.
    pub project: Arc<Project>,
    /// Where on the timeline.
    pub timeline_time: Timestamp,
    /// The largest size the interface can show.
    pub maximum_size: PictureSize,
    /// Where the encoded frame goes.
    pub reply: Sender<PictureReply>,
}

/// A request for a thumbnail.
pub struct ThumbnailJob {
    /// The video or picture file.
    pub file_path: PathBuf,
    /// Time from the start of the file.
    pub source_time: Timestamp,
    /// The largest thumbnail size.
    pub maximum_size: PictureSize,
    /// Where the encoded thumbnail goes.
    pub reply: Sender<PictureReply>,
}

/// What a picture thread is asked.
enum WorkerRequest<Job> {
    /// A picture.
    Picture(Job),
    /// Close every open file, then answer.
    CloseFiles(Sender<()>),
}

/// How long [`PreviewWorker::close_files`] and [`ThumbnailWorker::close_files`]
/// wait for the thread, which may be busy with a picture.
const CLOSE_FILES_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Asks a picture thread to close its files and waits for it.
fn ask_to_close_files<Job>(jobs: &Sender<WorkerRequest<Job>>) {
    let (answer, answered) = mpsc::channel();
    if jobs.send(WorkerRequest::CloseFiles(answer)).is_ok() {
        let _ = answered.recv_timeout(CLOSE_FILES_TIMEOUT);
    }
}

/// Sends jobs to the preview thread.
pub struct PreviewWorker {
    jobs: Sender<WorkerRequest<PreviewJob>>,
}

impl PreviewWorker {
    /// Starts the preview thread, which shows videos from the proxies listed in
    /// `proxies` when there are some.
    ///
    /// # Errors
    ///
    /// When the system cannot start a thread.
    pub fn start(
        backend: Arc<dyn MediaBackend>,
        proxies: Arc<ReadyProxies>,
    ) -> std::io::Result<Self> {
        let (jobs, received_jobs) = mpsc::channel();
        thread::Builder::new()
            .name("kadr-preview".to_string())
            .spawn(move || run_preview_thread(backend.as_ref(), proxies, &received_jobs))?;
        Ok(Self { jobs })
    }

    /// Closes every file the preview holds open (they reopen on the next frame).
    /// Blocks for a moment: call it off the interface's thread.
    pub fn close_files(&self) {
        ask_to_close_files(&self.jobs);
    }

    /// Asks for a frame; the answer arrives on the returned channel.
    pub fn request_frame(
        &self,
        project: Arc<Project>,
        timeline_time: Timestamp,
        maximum_size: PictureSize,
    ) -> Receiver<PictureReply> {
        let (reply, answer) = mpsc::channel();
        let job = PreviewJob {
            project,
            timeline_time,
            maximum_size,
            reply,
        };
        if let Err(mpsc::SendError(WorkerRequest::Picture(job))) =
            self.jobs.send(WorkerRequest::Picture(job))
        {
            let _ = job.reply.send(Err(worker_stopped("preview")));
        }
        answer
    }
}

/// Sends jobs to the thumbnail thread.
pub struct ThumbnailWorker {
    jobs: Sender<WorkerRequest<ThumbnailJob>>,
}

impl ThumbnailWorker {
    /// Starts the thumbnail thread, which reads videos from the proxies listed
    /// in `proxies` when there are some.
    ///
    /// # Errors
    ///
    /// When the system cannot start a thread.
    pub fn start(
        backend: Arc<dyn MediaBackend>,
        proxies: Arc<ReadyProxies>,
    ) -> std::io::Result<Self> {
        let (jobs, received_jobs) = mpsc::channel();
        thread::Builder::new()
            .name("kadr-thumbnails".to_string())
            .spawn(move || run_thumbnail_thread(backend.as_ref(), proxies, &received_jobs))?;
        Ok(Self { jobs })
    }

    /// Closes every file the thumbnail thread holds open. Blocks for a moment:
    /// call it off the interface's thread.
    pub fn close_files(&self) {
        ask_to_close_files(&self.jobs);
    }

    /// Asks for a thumbnail; the answer arrives on the returned channel.
    pub fn request_thumbnail(
        &self,
        file_path: PathBuf,
        source_time: Timestamp,
        maximum_size: PictureSize,
    ) -> Receiver<PictureReply> {
        let (reply, answer) = mpsc::channel();
        let job = ThumbnailJob {
            file_path,
            source_time,
            maximum_size,
            reply,
        };
        if let Err(mpsc::SendError(WorkerRequest::Picture(job))) =
            self.jobs.send(WorkerRequest::Picture(job))
        {
            let _ = job.reply.send(Err(worker_stopped("thumbnail")));
        }
        answer
    }
}

fn worker_stopped(worker_name: &str) -> CommandError {
    CommandError::internal(format!("the {worker_name} thread has stopped"))
}

fn run_preview_thread(
    backend: &dyn MediaBackend,
    proxies: Arc<ReadyProxies>,
    jobs: &Receiver<WorkerRequest<PreviewJob>>,
) {
    // `GpuIfAvailable` falls back to the CPU instead of failing.
    let mut renderer = match PreviewRenderer::new(RendererPreference::GpuIfAvailable) {
        Ok(renderer) => renderer,
        Err(error) => {
            answer_every_job_with_error(jobs, &error.to_string());
            return;
        }
    };
    renderer.use_proxies(proxies);
    while let Ok(request) = jobs.recv() {
        let mut job = match request {
            WorkerRequest::Picture(job) => job,
            WorkerRequest::CloseFiles(answer) => {
                renderer.close_open_files();
                let _ = answer.send(());
                continue;
            }
        };
        while let Ok(newer_request) = jobs.try_recv() {
            match newer_request {
                WorkerRequest::Picture(newer_job) => {
                    let _ = job.reply.send(Err(CommandError::new(
                        ErrorCode::Superseded,
                        "a newer frame was asked for",
                    )));
                    job = newer_job;
                }
                WorkerRequest::CloseFiles(answer) => {
                    renderer.close_open_files();
                    let _ = answer.send(());
                }
            }
        }
        let result = renderer
            .render_frame(&job.project, backend, job.timeline_time, job.maximum_size)
            .map(|picture| encode_yuv_picture(&picture))
            .map_err(|error| CommandError::new(ErrorCode::PreviewFailed, error));
        let _ = job.reply.send(result);
    }
}

fn answer_every_job_with_error(jobs: &Receiver<WorkerRequest<PreviewJob>>, details: &str) {
    while let Ok(request) = jobs.recv() {
        match request {
            WorkerRequest::Picture(job) => {
                let _ = job
                    .reply
                    .send(Err(CommandError::new(ErrorCode::PreviewFailed, details)));
            }
            WorkerRequest::CloseFiles(answer) => {
                let _ = answer.send(());
            }
        }
    }
}

fn run_thumbnail_thread(
    backend: &dyn MediaBackend,
    proxies: Arc<ReadyProxies>,
    jobs: &Receiver<WorkerRequest<ThumbnailJob>>,
) {
    let mut thumbnail_maker = ThumbnailMaker::new();
    thumbnail_maker.use_proxies(proxies);
    while let Ok(request) = jobs.recv() {
        let job = match request {
            WorkerRequest::Picture(job) => job,
            WorkerRequest::CloseFiles(answer) => {
                thumbnail_maker.close_open_files();
                let _ = answer.send(());
                continue;
            }
        };
        let result = thumbnail_maker
            .thumbnail_at(backend, &job.file_path, job.source_time, job.maximum_size)
            .map(|thumbnail| {
                encode_rgba_picture(thumbnail.width(), thumbnail.height(), thumbnail.as_raw())
            })
            .map_err(|error| {
                CommandError::new(ErrorCode::PreviewFailed, error).about_file(&job.file_path)
            });
        let _ = job.reply.send(result);
    }
}
