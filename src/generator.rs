//! Possible constraints
//!  - Season / date range
//!  - Area
//!  - Tasks that must be done on specific date
//!  - Priority levels: higher priority => don't let task be overdue
//!
//! Other features
//!  - Allow multiple on the same day. Don't let total effort exceed limit.
//!

use std::cmp::{Ordering, Reverse};

use chrono::{Datelike, Duration, NaiveDate};
use fnv::FnvHashSet;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

use crate::{
    loader::{effort, Season, TaskLog, TaskRow},
    task::{DatedTask, Task},
};

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

    let start_date = if let Some(most_recent_log) = history.first() {
        most_recent_log.date + Duration::days(1)
    } else {
        // No history, default to Jan 1st
        NaiveDate::from_ymd_opt(current_year, 1, 1).unwrap()
    };

    if start_date.year() > current_year {
        panic!("History is in the future or too recent");
    }
    let oldest_date = history.last().map(|log| log.date).unwrap_or(start_date);

    // Convert TaskRow -> Task
    let mut tasks: Vec<Task> = task_rows.iter().map(From::from).collect();

    let mut used_days = FnvHashSet::default();

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
            task.seen = true;
            used_days.insert(task.days_until);
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

    let mut cleaning_list = Vec::new();

    let mut rng = if let Some(seed) = seed {
        ChaCha8Rng::seed_from_u64(seed)
    } else {
        ChaCha8Rng::from_rng(&mut rand::rng())
    };

    // Sort by season (summer first), then period
    tasks.sort_by_key(|t| t.period_days);

    tasks = tasks
        .iter()
        .filter(|t| t.season == Season::Summer)
        .chain(tasks.iter().filter(|t| t.season == Season::Any))
        .cloned()
        .collect();

    // Set `days_until` to determine when the first occurence of each task should be
    for task in tasks.iter_mut().filter(|t| !t.seen) {
        task.set_days_until(&mut rng, &mut used_days, oldest_date, start_date);
    }

    // tasks.sort_by_key(|t| t.days_until);
    // tasks.sort_by_key(|t| t.period_days);
    // for t in &tasks {
    //     println!("{:>3}  {}  {}", t.id, t.period_days, t.days_until);
    // }

    let mut total_error = 0.0;
    let mut max_days_until = i32::MIN;
    let mut min_days_until = i32::MAX;

    // Helper to add task to cleaning list
    let mut add_task = |task: &mut Task, date: NaiveDate| {
        if task.seen {
            total_error += task.slack.abs();
            max_days_until = max_days_until.max(task.days_until);
            min_days_until = min_days_until.min(task.days_until);
        } else {
            task.days_until = 0;
        }

        cleaning_list.push(DatedTask::from_task(task, date));
        task.days_until = task.period_days; // must be updated AFTER adding to list
        task.seen = true;
    };

    let mut prev_effort = 0;

    // Exactly one task per day (counting 'day off' as a task)
    for current_date in start_date.iter_days().take_while(|&d| d <= end_date) {
        // Daily update
        for task in tasks.iter_mut() {
            task.days_until -= 1;
            update_slack(task);
        }

        if holidays.contains(&current_date) {
            add_task(&mut Task::get_holiday_task(), current_date);
            prev_effort = 0;
            continue;
        }

        let next_task = get_valid_tasks(&mut tasks, current_date, prev_effort)
            .into_iter()
            .min_by(lowest_slack)
            .expect("need at least 1 valid task");

        add_task(next_task, current_date);
        prev_effort = next_task.effort;
    }

    dbg!(total_error, max_days_until, min_days_until);

    cleaning_list
}

/// This helper should be called every day for every task to update `days_until` and `slack`.
fn update_slack(task: &mut Task) {
    /// Task `slack` is the signed squared error (error is `days_until`) multiplied by the inverse
    /// `period_days` raised to `PERIOD_POWER`. So setting this to 1.0 would make the slack proportional
    /// to the inverse of `period_days`, but that doesn't create quite enough difference between low
    /// and high period tasks. The exact value is arbitrary but 1.5 seems to produce well balanced
    /// output.
    const PERIOD_POWER: f32 = 1.5; // ;)

    // Weight by period. This way shorter period tasks are prioritized more when overdue, and
    // longer period tasks are prioritized when no tasks are due.
    let weight = 1.0 / (task.period_days as f32).powf(PERIOD_POWER);
    let signed_sq_err = task.days_until as f32 * (task.days_until as f32).abs();
    task.slack = signed_sq_err * weight;
}

/// Order by slack, then period
fn lowest_slack(a: &&mut Task, b: &&mut Task) -> Ordering {
    match a.slack.total_cmp(&b.slack) {
        Ordering::Equal => a.period_days.cmp(&b.period_days),
        ord => ord,
    }
}

/// Applies constraints:
///  - Don't do high effort tasks back to back
///  - Don't do summer tasks outside of summer
///  - Prioritize summer tasks that are due before end of summer
fn get_valid_tasks(tasks: &mut [Task], current_date: NaiveDate, prev_effort: u8) -> Vec<&mut Task> {
    let max_effort = if prev_effort >= effort::HARD {
        effort::MODERATE
    } else {
        effort::EXTREME
    };
    let mut valid_tasks: Vec<&mut Task> = tasks
        .iter_mut()
        .filter(|t| t.effort <= max_effort)
        .collect();

    if Season::SUMMER_WINDOW.contains(current_date) {
        let summer_end = Season::SUMMER_WINDOW
            .get_end_date(current_date.year())
            .unwrap();
        let days_left = (summer_end - current_date).num_days() as i32;
        assert!(days_left >= 0);

        // Summer tasks that are due before end of summer get moved to the front
        valid_tasks.sort_by(lowest_slack);
        valid_tasks.sort_by_key(|t| t.season != Season::Summer || t.days_until > days_left);

        // This ensures that any summer tasks that are due before end of summer will be completed
        // (does not account for holidays)
        valid_tasks.truncate(days_left as usize + 1);
    } else {
        valid_tasks.retain(|t| t.season != Season::Summer);
    }
    valid_tasks
}

#[cfg(test)]
mod test {
    use chrono::Duration;

    use crate::generator::{get_valid_tasks, update_slack};
    use crate::loader::Season;
    use crate::task::Task;

    #[test]
    fn urgent_summer_tasks() {
        let mut tasks = [
            Task::new(1, 365, 2).with_season(Season::Summer),
            Task::new(2, 7, 3).with_season(Season::Summer),
            Task::new(3, 10, 0),
            Task::new(4, 7, -1),
            Task::new(5, 12, 0),
        ];

        tasks.iter_mut().for_each(update_slack);

        // 4 days of summer left
        let summer_end = Season::SUMMER_WINDOW.get_end_date(2026).unwrap();
        let current_date = summer_end - Duration::days(3);

        let valid_tasks = get_valid_tasks(&mut tasks, current_date, 0);
        let ids = valid_tasks.iter().map(|t| t.id).collect::<Vec<_>>();

        // Since there are only 4 days of summer left, valid_tasks should include the 2 summer tasks
        // even though they have higher slack (they are due before end of summer) and it should only
        // include 2 other tasks (2+2=4).
        assert_eq!(&ids, &[1, 2, 4, 3]);
    }
}
