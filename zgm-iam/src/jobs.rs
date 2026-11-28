//! The loop of the module: `build()` starts it, and every two minutes it deletes, in batches,
//! the users and then the tenants whose date of deletion has come. Its interval and batches are
//! constants of the module, as in `zgm-limits`.

use std::panic::{self, AssertUnwindSafe};
use std::time::Duration;

use tokio::task::AbortHandle;

use crate::error::IamError;
use crate::iam::Iam;
use crate::repository;

/// How many users one batch deletes at most. A batch holds its rows until it is done, and a
/// change of one of those users waits for it; `zgm-limits` measured that a hundred scattered
/// rows keep that wait to a fraction of a second, where a thousand took one to two seconds.
pub(crate) const BATCH_SIZE: i64 = 100;

/// How many tenants one batch deletes: one, as the database deletes the users of a tenant with
/// it in the same statement. Measured on 2026-11-28, a tenant of 50,000 users went in 53 ms and
/// one of 500,000 in 187 ms; one at a time, many large tenants with the same date do not go in
/// one statement.
pub(crate) const TENANT_BATCH_SIZE: i64 = 1;

/// The time between the passes of the loop: a user is gone at most two minutes after its
/// date. A pass with nothing to delete is one short query on the index of the dates.
pub(crate) const PASS: Duration = Duration::from_mins(2);

/// The pause between two full batches of one pass, so that a large backlog, e.g. 100,000
/// users deleted at once, goes at about 300 users a second and does not keep the database busy.
pub(crate) const BATCH_PAUSE: Duration = Duration::from_millis(300);

/// A job of the loop. The application matches on it, so a new job breaks its `match` instead
/// of passing unseen. It prints as the name of the job, e.g. `delete_users_past_their_date`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
#[strum(serialize_all = "snake_case")]
pub enum Job {
    /// Deleting the users whose date of deletion has come, in every tenant, every two
    /// minutes.
    DeleteUsersPastTheirDate,
    /// Deleting the tenants whose date of deletion has come, with their users, every two
    /// minutes.
    DeleteTenantsPastTheirDate,
}

impl Job {
    /// The jobs in the order a pass runs them.
    const ALL: [Job; 2] = [
        Job::DeleteUsersPastTheirDate,
        Job::DeleteTenantsPastTheirDate,
    ];

    /// How many rows one batch of the job deletes at most.
    fn batch_size(self) -> i64 {
        match self {
            Job::DeleteUsersPastTheirDate => BATCH_SIZE,
            Job::DeleteTenantsPastTheirDate => TENANT_BATCH_SIZE,
        }
    }
}

/// A job of the loop that failed. The loop goes on and tries the job again in its next pass.
#[derive(Debug)]
pub struct JobError {
    /// Which job failed.
    pub job: Job,
    /// Why.
    pub error: IamError,
}

/// What the application does with a [`JobError`]: the function it gives to
/// `run_jobs_in_background`.
pub(crate) type OnJobError = Box<dyn Fn(JobError) + Send + Sync>;

/// Stops the loop when the last clone of `Iam` that holds it goes away.
pub(crate) struct StopJobsOnDrop(pub(crate) AbortHandle);

impl Drop for StopJobsOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl Iam {
    /// The loop: a pass at once, then one every two minutes. A pass that fails goes to
    /// `on_error` and runs again next time; the loop never stops on its own, not even when
    /// `on_error` panics.
    ///
    /// `self` here is the loop's own copy of the module, which holds no `StopJobsOnDrop`: with
    /// one it would keep itself running for ever.
    pub(crate) async fn run_jobs(self, on_error: OnJobError) {
        loop {
            self.run_pass(&on_error).await;
            tokio::time::sleep(PASS).await;
        }
    }

    /// One pass: every job in turn. A job that fails does not keep the next from running.
    async fn run_pass(&self, on_error: &OnJobError) {
        for job in Job::ALL {
            self.run_job(job, on_error).await;
        }
    }

    /// One job of a pass: batches until one is not full, or fails. A full batch means there
    /// may be more, so the next one follows after a short pause.
    async fn run_job(&self, job: Job, on_error: &OnJobError) {
        loop {
            match self.delete_batch(job).await {
                Ok(deleted) if deleted == job.batch_size() as u64 => {
                    tokio::time::sleep(BATCH_PAUSE).await;
                }
                Ok(_) => break,
                Err(error) => {
                    let job_error = JobError { job, error };
                    // The application's function must not be able to end the loop: a panic in
                    // it would kill this task for good, and nothing would say so. The panic is
                    // dropped here; the function had its chance to report.
                    let _panic = panic::catch_unwind(AssertUnwindSafe(|| on_error(job_error)));
                    break;
                }
            }
        }
    }

    /// Deletes one batch of the job: of the users, or of the tenants, whose date of deletion
    /// has come.
    async fn delete_batch(&self, job: Job) -> Result<u64, IamError> {
        let mut connection = self.pool().acquire().await?;
        match job {
            Job::DeleteUsersPastTheirDate => {
                repository::delete_users_past_their_date(&mut connection, job.batch_size()).await
            }
            Job::DeleteTenantsPastTheirDate => {
                repository::delete_tenants_past_their_date(&mut connection, job.batch_size()).await
            }
        }
    }
}
