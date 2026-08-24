//! CronJob & Scheduled Task Manager for VeloceNetwork (v3.1).
//!
//! Provides scheduled task orchestration supporting standard cron syntax,
//! interval tokens, concurrency policies, and execution history tracking.

use parking_lot::RwLock;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{info, warn};
use uuid::Uuid;
use veloce_ipc::message::CronJobMsg;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConcurrencyPolicy {
    Allow,
    Forbid,
    Replace,
}

impl FromStr for ConcurrencyPolicy {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "forbid" => Ok(Self::Forbid),
            "replace" => Ok(Self::Replace),
            _ => Ok(Self::Allow),
        }
    }
}

impl ConcurrencyPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Allow => "Allow",
            Self::Forbid => "Forbid",
            Self::Replace => "Replace",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScheduleKind {
    Interval(Duration),
    Cron {
        minute: u32,
        hour: u32,
        is_wildcard_hour: bool,
    },
}

impl ScheduleKind {
    pub fn parse(expr: &str) -> Result<Self, String> {
        let trimmed = expr.trim();
        if let Some(rest) = trimmed.strip_prefix("@every ") {
            let secs: u64 = rest
                .trim_end_matches('s')
                .trim_end_matches("sec")
                .trim_end_matches("secs")
                .trim()
                .parse()
                .map_err(|e| format!("invalid interval: {}", e))?;
            return Ok(Self::Interval(Duration::from_secs(secs.max(1))));
        }

        match trimmed.to_ascii_lowercase().as_str() {
            "@hourly" => Ok(Self::Interval(Duration::from_secs(3600))),
            "@daily" => Ok(Self::Interval(Duration::from_secs(86400))),
            "@weekly" => Ok(Self::Interval(Duration::from_secs(604800))),
            _ => {
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                if parts.len() == 5 {
                    let min_part = parts[0];
                    let hour_part = parts[1];
                    let min = if min_part == "*" {
                        0
                    } else if let Some(step) = min_part.strip_prefix("*/") {
                        let step_num: u64 = step.parse().unwrap_or(1).max(1);
                        return Ok(Self::Interval(Duration::from_secs(step_num * 60)));
                    } else {
                        min_part.parse().unwrap_or(0)
                    };

                    let is_wildcard_hour = hour_part == "*";
                    let hour = if is_wildcard_hour { 0 } else { hour_part.parse().unwrap_or(0) };

                    Ok(Self::Cron {
                        minute: min,
                        hour,
                        is_wildcard_hour,
                    })
                } else {
                    Err(format!("invalid schedule expression: '{}'", expr))
                }
            }
        }
    }

    pub fn compute_next_run(&self, from_secs: u64) -> u64 {
        match self {
            Self::Interval(dur) => from_secs + dur.as_secs(),
            Self::Cron { minute, hour, is_wildcard_hour } => {
                let current_min = (from_secs / 60) % 60;

                if *is_wildcard_hour {
                    let diff_mins = if *minute > current_min as u32 {
                        *minute - current_min as u32
                    } else {
                        60 - (current_min as u32 - *minute)
                    };
                    from_secs + (diff_mins as u64 * 60)
                } else {
                    let target_sec_of_day = (*hour as u64 * 3600) + (*minute as u64 * 60);
                    let current_sec_of_day = from_secs % 86400;
                    if target_sec_of_day > current_sec_of_day {
                        from_secs + (target_sec_of_day - current_sec_of_day)
                    } else {
                        from_secs + (86400 - (current_sec_of_day - target_sec_of_day))
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CronJob {
    pub name: String,
    pub schedule_raw: String,
    pub schedule: ScheduleKind,
    pub executable: String,
    pub args: Vec<String>,
    pub concurrency_policy: ConcurrencyPolicy,
    pub enabled: bool,
    pub last_run_timestamp_secs: Option<u64>,
    pub last_run_status: Option<String>,
    pub next_run_timestamp_secs: Option<u64>,
    pub active_node_id: Option<Uuid>,
}

impl CronJob {
    pub fn from_msg(msg: CronJobMsg) -> Result<Self, String> {
        let schedule = ScheduleKind::parse(&msg.schedule)?;
        let policy = ConcurrencyPolicy::from_str(&msg.concurrency_policy).unwrap_or(ConcurrencyPolicy::Allow);
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let next_run = Some(schedule.compute_next_run(now_secs));

        Ok(Self {
            name: msg.name,
            schedule_raw: msg.schedule,
            schedule,
            executable: msg.executable,
            args: msg.args,
            concurrency_policy: policy,
            enabled: msg.enabled,
            last_run_timestamp_secs: msg.last_run_timestamp_secs,
            last_run_status: msg.last_run_status,
            next_run_timestamp_secs: next_run,
            active_node_id: None,
        })
    }

    pub fn to_msg(&self) -> CronJobMsg {
        CronJobMsg {
            name: self.name.clone(),
            schedule: self.schedule_raw.clone(),
            executable: self.executable.clone(),
            args: self.args.clone(),
            concurrency_policy: self.concurrency_policy.as_str().to_string(),
            enabled: self.enabled,
            last_run_timestamp_secs: self.last_run_timestamp_secs,
            last_run_status: self.last_run_status.clone(),
            next_run_timestamp_secs: self.next_run_timestamp_secs,
        }
    }
}

pub struct CronScheduler {
    jobs: RwLock<HashMap<String, CronJob>>,
}

impl CronScheduler {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            jobs: RwLock::new(HashMap::new()),
        })
    }

    pub fn add_job(&self, job: CronJob) {
        info!(
            job = %job.name,
            schedule = %job.schedule_raw,
            policy = job.concurrency_policy.as_str(),
            "cron: registered scheduled task"
        );
        self.jobs.write().insert(job.name.clone(), job);
    }

    pub fn remove_job(&self, name: &str) -> bool {
        let removed = self.jobs.write().remove(name).is_some();
        if removed {
            info!(job = %name, "cron: task deleted");
        }
        removed
    }

    pub fn get_job(&self, name: &str) -> Option<CronJob> {
        self.jobs.read().get(name).cloned()
    }

    pub fn list_jobs(&self) -> Vec<CronJob> {
        self.jobs.read().values().cloned().collect()
    }

    /// Checks which enabled jobs are due for execution at `now_secs`.
    /// Updates their `next_run_timestamp_secs` and returns the due jobs.
    pub fn poll_due_jobs(&self, now_secs: u64) -> Vec<CronJob> {
        let mut due = Vec::new();
        let mut jobs = self.jobs.write();

        for job in jobs.values_mut() {
            if !job.enabled {
                continue;
            }

            if let Some(next_run) = job.next_run_timestamp_secs {
                if now_secs >= next_run {
                    // Check concurrency policy
                    if job.active_node_id.is_some() && job.concurrency_policy == ConcurrencyPolicy::Forbid {
                        warn!(job = %job.name, "cron: skipping execution due to Forbid concurrency policy");
                        job.next_run_timestamp_secs = Some(job.schedule.compute_next_run(now_secs));
                        continue;
                    }

                    job.last_run_timestamp_secs = Some(now_secs);
                    job.last_run_status = Some("Running".to_string());
                    job.next_run_timestamp_secs = Some(job.schedule.compute_next_run(now_secs));
                    due.push(job.clone());
                }
            }
        }

        due
    }

    pub fn record_start(&self, name: &str, node_id: Uuid) {
        if let Some(job) = self.jobs.write().get_mut(name) {
            job.active_node_id = Some(node_id);
        }
    }

    pub fn record_completion(&self, name: &str, success: bool) {
        if let Some(job) = self.jobs.write().get_mut(name) {
            job.active_node_id = None;
            job.last_run_status = Some(if success { "Success".to_string() } else { "Failed".to_string() });
            info!(job = %name, success = success, "cron: task execution finished");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schedule_interval_parsing() {
        let sched = ScheduleKind::parse("@every 30s").unwrap();
        assert_eq!(sched, ScheduleKind::Interval(Duration::from_secs(30)));

        let sched_step = ScheduleKind::parse("*/5 * * * *").unwrap();
        assert_eq!(sched_step, ScheduleKind::Interval(Duration::from_secs(300)));
    }

    #[test]
    fn test_schedule_cron_parsing() {
        let sched = ScheduleKind::parse("15 3 * * *").unwrap();
        match sched {
            ScheduleKind::Cron { minute, hour, is_wildcard_hour } => {
                assert_eq!(minute, 15);
                assert_eq!(hour, 3);
                assert!(!is_wildcard_hour);
            }
            _ => panic!("expected Cron schedule"),
        }
    }

    #[test]
    fn test_cron_poll_due_and_concurrency() {
        let scheduler = CronScheduler::new();
        let job = CronJob {
            name: "backup".into(),
            schedule_raw: "@every 10s".into(),
            schedule: ScheduleKind::Interval(Duration::from_secs(10)),
            executable: "./backup.sh".into(),
            args: vec![],
            concurrency_policy: ConcurrencyPolicy::Forbid,
            enabled: true,
            last_run_timestamp_secs: None,
            last_run_status: None,
            next_run_timestamp_secs: Some(100),
            active_node_id: None,
        };
        scheduler.add_job(job);

        // Not due yet at t=90
        let due = scheduler.poll_due_jobs(90);
        assert!(due.is_empty());

        // Due at t=100
        let due = scheduler.poll_due_jobs(100);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].name, "backup");

        // Mark as actively running
        scheduler.record_start("backup", Uuid::new_v4());

        // Next poll while still active with Forbid should skip
        let due2 = scheduler.poll_due_jobs(115);
        assert!(due2.is_empty());

        // Complete job
        scheduler.record_completion("backup", true);
        assert_eq!(scheduler.get_job("backup").unwrap().last_run_status.as_deref(), Some("Success"));
    }
}
