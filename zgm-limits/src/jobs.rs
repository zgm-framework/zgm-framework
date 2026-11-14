//! The loop of the module's cleanup: `build()` starts it, and every two minutes it deletes, in
//! batches, the counters that mean nothing any more. Its interval and batch are constants of
//! the module.

use std::panic::{self, AssertUnwindSafe};
use std::time::Duration;

use tokio::task::AbortHandle;

use crate::audit::LimitsEvent;
use crate::error::LimitsError;
use crate::limits::Limits;
use crate::repository;

/// How many counters one batch of the cleanup deletes at most. A batch holds its rows until it
/// is done, and an attempt on a key that comes back just then waits for it. Dead rows scattered
/// over the table take about a millisecond or two each to delete, so a hundred usually keeps
/// that wait to a fraction of a second, where a thousand took one to two seconds. On a server
/// under heavy load the slowest batches of a hundred were still seen to take a second or two.
pub(crate) const BATCH_SIZE: i64 = 100;

/// The time between the passes of the loop. A pass with nothing to delete costs one short
/// query on the index of `expires_at`.
pub(crate) const PASS: Duration = Duration::from_mins(2);

/// The pause between two full batches of one pass. With it the cleanup deletes up to about 300
/// counters a second. When more than that become deletable, the table grows for a while and
/// the cleanup catches up when the traffic falls; a dead row costs room, nothing else.
pub(crate) const BATCH_PAUSE: Duration = Duration::from_millis(300);

/// How long a counter stays in the table after its `expires_at` before the cleanup deletes it.
///
/// Every copy of the application counts by its own clock. Without this margin a copy whose
/// clock runs five minutes fast would delete a block five minutes before its end, for
/// everybody. An hour is far more than clocks of servers differ by, and a row past its
/// `expires_at` counts for nothing in the meantime.
pub(crate) const KEPT_AFTER_THE_END: Duration = Duration::from_hours(1);

/// A job of the loop. The application matches on it, so a new job breaks its `match` instead
/// of passing unseen. It prints as the name of the job, e.g. `delete_expired`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
#[strum(serialize_all = "snake_case")]
pub enum Job {
    /// Deleting the counters that have been past their `expires_at` for an hour, every two
    /// minutes.
    DeleteExpired,
}

/// A job of the loop that failed. The loop goes on and tries the job again in its next pass.
#[derive(Debug)]
pub struct JobError {
    /// Which job failed.
    pub job: Job,
    /// Why.
    pub error: LimitsError,
}

/// What the application does with a [`JobError`]: the function it gives to
/// `run_jobs_in_background`.
pub(crate) type OnJobError = Box<dyn Fn(JobError) + Send + Sync>;

/// Stops the loop when the last clone of `Limits` that holds it goes away.
pub(crate) struct StopJobsOnDrop(pub(crate) AbortHandle);

impl Drop for StopJobsOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl Limits {
    /// The loop of the cleanup: a pass at once, then one every two minutes. A pass that fails
    /// goes to `on_error` and runs again next time; the loop never stops on its own, not even
    /// when `on_error` panics.
    ///
    /// `self` here is the loop's own copy of the module, which holds no `StopJobsOnDrop`: with
    /// one it would keep itself running for ever.
    pub(crate) async fn run_jobs(self, on_error: OnJobError) {
        loop {
            self.run_pass(&on_error).await;
            tokio::time::sleep(PASS).await;
        }
    }

    /// One pass: batches of the cleanup until one is not full, or fails. A full batch means
    /// there may be more, so the next one follows after a short pause.
    async fn run_pass(&self, on_error: &OnJobError) {
        let mut counters = 0;
        loop {
            match self.delete_expired().await {
                // A full batch: there may be more. A short pause first, so that a large
                // backlog is not deleted in one burst that slows the attempts down.
                Ok(deleted) if deleted == BATCH_SIZE as u64 => {
                    counters += deleted;
                    tokio::time::sleep(BATCH_PAUSE).await;
                }
                Ok(deleted) => {
                    counters += deleted;
                    break;
                }
                Err(error) => {
                    let job_error = JobError {
                        job: Job::DeleteExpired,
                        error,
                    };
                    // The application's function must not be able to end the cleanup: a panic
                    // in it would kill this task for good, and nothing would say so. The
                    // panic is dropped here; the function had its chance to report.
                    let _panic = panic::catch_unwind(AssertUnwindSafe(|| on_error(job_error)));
                    break;
                }
            }
        }
        // One event for the whole pass, and none for a pass that found nothing.
        if counters > 0 {
            self.emit(LimitsEvent::ExpiredDeleted { counters });
        }
    }

    /// Deletes one batch of the counters that have been past their `expires_at` for
    /// [`KEPT_AFTER_THE_END`]. Without it the limits still
    /// count correctly, only the table grows: a counter past its `expires_at` means nothing.
    async fn delete_expired(&self) -> Result<u64, LimitsError> {
        let mut connection = self.pool().acquire().await?;
        let dead_since = self.now() - KEPT_AFTER_THE_END;
        Ok(repository::delete_expired(&mut connection, dead_since, BATCH_SIZE).await?)
    }
}
