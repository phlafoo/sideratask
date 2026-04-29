use std::{cmp::Reverse, ops::Deref};

use chrono::{Datelike, Duration, NaiveDate, TimeDelta};
use rand::{Rng, RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;

use crate::{Season, Task};

pub struct DatedTask {
    pub id: usize,
    pub do_date: NaiveDate,
    pub instructions: String,
}

impl DatedTask {
    fn new(id: usize, do_date: NaiveDate, instructions: String) -> Self {
        DatedTask {
            id,
            do_date,
            instructions,
        }
    }

    fn from_task(task: &Task, do_date: NaiveDate) -> Self {
        DatedTask {
            id: task.id,
            do_date,
            instructions: task.instructions.clone(),
        }
    }
}

impl Task {
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
pub fn generate_cleaning_list(mut tasks: Vec<Task>, start_date: NaiveDate) -> String {
    // Logic relies on always ending on Dec 31
    let year = start_date.year();
    let end_date = NaiveDate::from_ymd_opt(year, 12, 31).unwrap();
    assert!(start_date < end_date);

    let holidays = [
        NaiveDate::from_ymd_opt(year, 1, 1).unwrap(), // New years day
        NaiveDate::from_ymd_opt(year, 3, 27).unwrap(), // My bday
        NaiveDate::from_ymd_opt(year, 11, 17).unwrap(), // Averee bday
        NaiveDate::from_ymd_opt(year, 12, 25).unwrap(), // Christmas
    ];
    let total_days = (end_date - start_date).num_days() as i32;

    let tasks_per_day = tasks
        .iter()
        .map(|t| 1.0 / t.period_days as f32)
        .sum::<f32>();
    let holidays_per_day = holidays.len() as f32 / total_days as f32;
    let days_off_per_day = 1.0 - (tasks_per_day + holidays_per_day);
    if days_off_per_day > 0.0 {
        let days_off_period = (1.0 / days_off_per_day) as i32;
        tasks.push(Task::get_day_off_task(days_off_period));
    }

    let mut summer_start = NaiveDate::from_ymd_opt(year, 5, 1).unwrap();
    let mut summer_end = NaiveDate::from_ymd_opt(year, 8, 31).unwrap();

    // Need to guarantee:
    //  1. summer_start >= start_date
    //  2. summer_end > start_date
    if start_date >= summer_end {
        summer_end.with_year(year + 1);
        summer_start.with_year(year + 1);
    } else if start_date > summer_start {
        summer_start = start_date;
    }

    let days_until_summer_start = (summer_start - start_date).num_days() as i32;
    let days_until_summer_end = (summer_end - start_date).num_days() as i32;

    const SEED: u64 = 1;
    let mut rng = ChaCha8Rng::seed_from_u64(SEED);
    // let mut rng = ChaCha8Rng::from_rng(&mut rand::rng());

    for task in tasks.iter_mut().filter(|t| t.days_until == 0) {
        task.set_days_until(
            &mut rng,
            total_days,
            days_until_summer_start,
            days_until_summer_end,
        );
    }

    let mut cleaning_list = Vec::new();

    let mut add_task = |task: &mut Task, date: NaiveDate| {
        cleaning_list.push((DatedTask::from_task(task, date), task.days_until));
        task.days_until = task.period_days;
    };

    let mut prev_effort = 0;

    // Max one task per day
    for current_date in start_date.iter_days().take_while(|&d| d <= end_date) {
        for task in &mut tasks {
            task.days_until -= 1;
        }

        let mut dtasks = tasks.clone();
        dtasks.sort_by_key(|t| t.days_until);
        // println!(
        //     "{}",
        //     dtasks
        //         .iter()
        //         .take(10)
        //         .map(|t| format!("{:>3}", t.days_until))
        //         .collect::<Vec<String>>()
        //         .join(" ")
        // );

        if holidays.contains(&current_date) {
            add_task(&mut Task::get_holiday_task(), current_date);
            prev_effort = 0;
            continue;
        }

        let valid_tasks: Vec<&mut Task> = {
            // If yesterday's task was high effort (>=5), today's task should be at most effort 3
            let max_effort = if prev_effort >= 5 { 3 } else { 8 };
            let iter = tasks.iter_mut().filter(|t| t.effort <= max_effort);

            let is_summer = summer_start < current_date && current_date < summer_end;
            if is_summer {
                iter.collect()
            } else {
                iter.filter(|t| t.season != Season::Summer).collect()
            }
        };

        let min_days_until = valid_tasks
            .iter()
            .min_by_key(|t| t.days_until)
            .unwrap()
            .days_until;

        let urgent = min_days_until <= 0;

        // Any task with the minimum days_until is a candidate
        let candidate_iter = valid_tasks
            .into_iter()
            .filter(|t| t.days_until == min_days_until);

        // Smaller period tasks are less flexible than larger period tasks. So if any tasks are
        // urgent, select the candidate with the SMALLEST period - otherwise select the candidate
        // with the LARGEST period.
        let next_task = if urgent {
            candidate_iter.min_by_key(|t| t.period_days).unwrap()
        } else {
            candidate_iter.max_by_key(|t| t.period_days).unwrap()
        };
        add_task(next_task, current_date);
        prev_effort = next_task.effort;
    }

    // print_summary(&tasks, &cleaning_list);

    // Create TSV string (an extra tab is placed after date column for the checkbox column)
    cleaning_list
        .iter()
        .map(|task| {
            format!(
                "{}\t{}\t{}\t{}",
                task.0.id, task.0.do_date, task.1, task.0.instructions
            )
        })
        .collect::<Vec<String>>()
        .join("\n")
    // let tsv_output = cleaning_list
    //     .iter()
    //     .map(|task| format!("{}\t{}\t\t{}", task.id, task.do_date, task.instructions))
    //     .collect::<Vec<String>>()
    //     .join("\n");

    // cleaning_list
}

fn print_summary(tasks: &[Task], cleaning_list: &[(DatedTask, i32)]) {
    println!("period | count | task");
    println!("-------+-------+-----");
    for task in tasks.iter() {
        let count = cleaning_list.iter().filter(|dt| dt.0.id == task.id).count();
        println!(
            "{:>6} | {:>5} | {}",
            task.period_days, count, task.instructions
        );
    }
    println!("-------+-------+-----");
}
