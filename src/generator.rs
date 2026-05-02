use std::cmp::{Ordering, Reverse};

use chrono::{Datelike, Duration, NaiveDate};
use rand::{Rng, RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;

use crate::loader::{effort, Season, TaskLog, TaskRow};

/// Used in generator algorithm
#[derive(Clone, Debug)]
pub struct Task {
    id: usize,
    instructions: String,
    period_days: i32,
    effort: u8, // 1, 2, 3, 5, or 8
    season: Season,
    /// How many days until this task should be done again. Negative value mean it is overdue.
    days_until: i32,
    /// This combines days_until with period so that shorter period tasks are prioritized more when
    /// overdue, and longer period tasks are prioritized when no tasks are due. Lower values mean the
    /// task is more overdue.
    slack: f32,
}

impl Task {
    const DAY_OFF_ID: usize = 0;

    fn with_period(mut self, period_days: i32) -> Self {
        self.period_days = period_days;
        self
    }

    fn get_day_off_task() -> Self {
        Task {
            id: Task::DAY_OFF_ID,
            instructions: "Day off".to_string(),
            period_days: 0,
            effort: 0,
            season: Season::Any,
            days_until: 0,
            slack: 0.0,
        }
    }

    fn get_holiday_task() -> Self {
        let mut task = Task::get_day_off_task();
        task.instructions = "Holiday".to_string();
        task
    }
}

impl From<&TaskRow> for Task {
    fn from(value: &TaskRow) -> Self {
        Task {
            id: value.id,
            instructions: value.instructions.clone(),
            period_days: value.period_days,
            effort: value.effort,
            season: value.season,
            days_until: 0,
            slack: 0.0,
        }
    }
}

/// The final cleaning list uses this type
#[derive(Clone, Debug)]
pub struct DatedTask {
    pub id: usize,
    pub do_date: NaiveDate,
    pub instructions: String,
    /// Just for debugging
    #[allow(dead_code)]
    pub days_until_when_added: i32,
}

impl DatedTask {
    fn from_task(task: &Task, do_date: NaiveDate) -> Self {
        DatedTask {
            id: task.id,
            do_date,
            instructions: task.instructions.clone(),
            days_until_when_added: task.days_until,
        }
    }
}

impl Task {
    /// The first instance of a task should happen randomly between start_date and start_date + period
    fn set_days_until<R: Rng + ?Sized>(
        &mut self,
        rng: &mut R,
        total_days: i32,
        days_until_summer_start: i32,
        days_until_summer_end: i32,
    ) {
        // The task may be skipped if it isn't the most urgent task when close to the end of date range
        // (year or summer). This "runway" accounts for that.
        const RUNWAY_DAYS: i32 = 10;

        let days_until_range = match self.season {
            Season::Any => {
                let limit = (total_days - RUNWAY_DAYS).max(1);
                let max = self.period_days.min(limit);
                0..max
            }
            Season::Summer => {
                let limit = (days_until_summer_end - RUNWAY_DAYS).max(days_until_summer_start + 1);
                let max = (days_until_summer_start + self.period_days).min(limit);
                days_until_summer_start..max
            }
        };
        // This is the *day before* the first day, so we +1
        self.days_until = rng.random_range(days_until_range) + 1;
    }
}

/// Returns list of dated tasks
pub fn generate_cleaning_list(
    task_rows: &[TaskRow],
    holidays: &[NaiveDate],
    mut history: Vec<TaskLog>,
    current_year: i32,
    seed: Option<u64>,
) -> Vec<DatedTask> {
    // The start date will be the day after the most recent record
    history.sort_by_key(|t| Reverse(t.date));

    let start_date = if let Some(most_recent_record) = history.first() {
        most_recent_record.date + Duration::days(1)
    } else {
        // No history, default to Jan 1st
        NaiveDate::from_ymd_opt(current_year, 1, 1).unwrap()
    };

    if start_date.year() > current_year {
        panic!("History is in the future or too recent");
    }

    // Convert TaskRow -> Task
    let mut tasks: Vec<Task> = task_rows.iter().map(From::from).collect();

    // All that is needed to integrate history into the algorithm, is to set the days_until based on
    // the last time the task was done
    for task in tasks.iter_mut() {
        if task.id == Task::DAY_OFF_ID {
            panic!("Task ID 0 is reserved for implicit 'day off' task");
        }
        // Scan history in descending order of date to get the most recent one
        if let Some(task_record) = history.iter().find(|r| r.id == task.id) {
            let days_ago = (start_date - task_record.date).num_days() as i32;
            task.days_until = task.period_days - days_ago;
        }
    }

    // For now we always end on Dec 31
    let year = start_date.year();
    let end_date = NaiveDate::from_ymd_opt(year, 12, 31).unwrap();
    assert!(start_date < end_date);

    let total_days = (end_date - start_date).num_days() as i32;

    // For the 'days off' task we need to approximate the period based on the periods of other tasks
    let tasks_per_day = tasks
        .iter()
        .map(|t| 1.0 / t.period_days as f32)
        .sum::<f32>();
    let holidays_per_day = holidays.len() as f32 / total_days as f32;
    let days_off_per_day = 1.0 - (tasks_per_day + holidays_per_day);
    if days_off_per_day > 0.0 {
        let day_off_task = Task::get_day_off_task().with_period((1.0 / days_off_per_day) as i32);
        tasks.push(day_off_task);
    }

    let mut summer_start = NaiveDate::from_ymd_opt(year, 5, 1).unwrap();
    let mut summer_end = NaiveDate::from_ymd_opt(year, 8, 31).unwrap();

    // Need to guarantee:
    //  1. summer_start >= start_date
    //  2. summer_end > start_date
    if start_date >= summer_end {
        summer_end = summer_end.with_year(year + 1).unwrap();
        summer_start = summer_start.with_year(year + 1).unwrap();
    } else if start_date > summer_start {
        summer_start = start_date;
    }

    let days_until_summer_start = (summer_start - start_date).num_days() as i32;
    let days_until_summer_end = (summer_end - start_date).num_days() as i32;
    let mut cleaning_list = Vec::new();

    let mut rng = if let Some(seed) = seed {
        ChaCha8Rng::seed_from_u64(seed)
    } else {
        ChaCha8Rng::from_rng(&mut rand::rng())
    };

    // Set `days_until` to determine when the first occurence of each task should be
    for task in tasks.iter_mut().filter(|t| t.days_until == 0) {
        task.set_days_until(
            &mut rng,
            total_days,
            days_until_summer_start,
            days_until_summer_end,
        );
    }

    // Helper to add task to cleaning list
    let mut add_task = |task: &mut Task, date: NaiveDate| {
        cleaning_list.push(DatedTask::from_task(task, date));
        task.days_until = task.period_days; // must be updated AFTER adding to list
    };

    /// Task `slack` is the signed squared error (error is `days_until`) multiplied by the inverse
    /// `period_days` raised to `PERIOD_POWER`. So setting this to 1.0 would make the slack proportional
    /// to the inverse of `period_days`, but that doesn't create quite enough difference between low
    /// and high period tasks. The exact value is arbitrary but 1.5 seems to produce well balanced
    /// output.
    const PERIOD_POWER: f32 = 1.5; // ;)

    // This helper should be called every day for every task to update `days_until` and `slack`.
    let update_slack = |task: &mut Task| {
        task.days_until -= 1;

        // Weight by period. This way shorter period tasks are prioritized more when overdue, and
        // longer period tasks are prioritized when no tasks are due.
        let weight = 1.0 / (task.period_days as f32).powf(PERIOD_POWER);
        let signed_sq_err = task.days_until as f32 * (task.days_until as f32).abs();
        task.slack = signed_sq_err * weight;
    };

    let mut prev_effort = 0;

    // Exactly one task per day (counting 'day off' as a task)
    for current_date in start_date.iter_days().take_while(|&d| d <= end_date) {
        // Daily update
        tasks.iter_mut().for_each(update_slack);

        if holidays.contains(&current_date) {
            add_task(&mut Task::get_holiday_task(), current_date);
            prev_effort = 0;
            continue;
        }

        // Other constraints:
        //  - Don't do high effort tasks back to back
        //  - Don't do summer tasks outside of summer
        let valid_tasks: Vec<&mut Task> = {
            let max_effort = if prev_effort >= effort::HARD {
                effort::MODERATE
            } else {
                effort::EXTREME
            };
            let is_summer = summer_start < current_date && current_date < summer_end;

            tasks
                .iter_mut()
                .filter(|t| t.effort <= max_effort)
                .filter(|t| is_summer || t.season != Season::Summer)
                .collect()
        };

        // Pick lowest `slack` task. On tie, pick lower `period_days` task
        let next_task = valid_tasks
            .into_iter()
            .min_by(|a, b| match a.slack.total_cmp(&b.slack) {
                Ordering::Equal => a.period_days.cmp(&b.period_days),
                ord => ord,
            })
            .expect("need at least 1 valid task");

        add_task(next_task, current_date);
        prev_effort = next_task.effort;
    }

    cleaning_list
}
