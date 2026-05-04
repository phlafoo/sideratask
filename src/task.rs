use chrono::{Datelike, Duration, NaiveDate};
use fnv::FnvHashSet;
use rand::{seq::IndexedRandom, Rng};

use crate::loader::{Season, TaskRow};

/// Month => [1..=12]
/// Day => [1..=31]
pub struct AnnualWindow {
    pub start_month: u32,
    pub start_day: u32,
    pub end_month: u32,
    pub end_day: u32,
}

impl AnnualWindow {
    pub fn contains(&self, date: NaiveDate) -> bool {
        let current = (date.month(), date.day());
        let start = (self.start_month, self.start_day);
        let end = (self.end_month, self.end_day);

        // Tuples are compared lexicographically
        if start <= end {
            current >= start && current <= end
        } else {
            current >= start || current <= end
        }
    }

    pub const fn get_start_date(&self, year: i32) -> Option<NaiveDate> {
        NaiveDate::from_ymd_opt(year, self.start_month, self.start_day)
    }

    pub const fn get_end_date(&self, year: i32) -> Option<NaiveDate> {
        NaiveDate::from_ymd_opt(year, self.end_month, self.end_day)
    }
}

/// Used in generator algorithm
#[derive(Clone, Debug, Default)]
pub struct Task {
    pub id: usize,
    pub instructions: String,
    pub period_days: i32,
    pub effort: u8, // 1, 2, 3, 5, or 8
    pub season: Season,
    /// How many days until this task should be done again. Negative value mean it is overdue.
    pub days_until: i32,
    /// This combines days_until with period so that shorter period tasks are prioritized more when
    /// overdue, and longer period tasks are prioritized when no tasks are due. Lower values mean the
    /// task is more overdue.
    pub slack: f32,
    pub seen: bool,
}

impl Task {
    pub const DAY_OFF_ID: usize = 0;

    #[cfg(test)]
    pub fn new(id: usize, period_days: i32, days_until: i32) -> Self {
        Task {
            id,
            period_days,
            days_until,
            ..Default::default()
        }
    }

    #[cfg(test)]
    pub fn with_season(mut self, season: Season) -> Self {
        self.season = season;
        self
    }

    pub fn with_period(mut self, period_days: i32) -> Self {
        self.period_days = period_days;
        self
    }

    pub fn get_day_off_task() -> Self {
        Task {
            id: Task::DAY_OFF_ID,
            instructions: "Day off".to_string(),
            ..Default::default()
        }
    }

    pub fn get_holiday_task() -> Self {
        let mut task = Task::get_day_off_task();
        task.instructions = "Holiday".to_string();
        task
    }

    /// Sets `days_until` for new tasks. Assumes that the task has never been done before.
    pub fn set_days_until<R: Rng + ?Sized>(
        &mut self,
        rng: &mut R,
        used_days: &mut FnvHashSet<i32>,
        oldest_date: NaiveDate,
        start_date: NaiveDate,
    ) {
        // Get allowed date range based on season.
        // Task period may be > 365 so we may need multiple ranges.
        let ranges = match self.season {
            Season::Any => vec![0..=i32::MAX],
            Season::Summer => {
                let max_date = start_date + Duration::days(self.period_days as i64);
                let window = &Season::SUMMER_WINDOW;
                (start_date.year()..=max_date.year())
                    .map(|year| {
                        let summer_start = window.get_start_date(year).unwrap();
                        let summer_end = window.get_end_date(year).unwrap();
                        (summer_start - start_date).num_days() as i32
                            ..=(summer_end - start_date).num_days() as i32
                    })
                    .collect::<Vec<_>>()
            }
        };
        // Since this task has never been done, the possible window should be truncated by `date_offset`.
        // e.g. if history goes back 10 days, and this task has a 30 day period, then the window will
        // be 0 to 20 days.
        let date_offset = (start_date - oldest_date).num_days() as i32;
        let days_within_period = 1..=(self.period_days - date_offset);

        // Filter for season
        let possible_days =
            days_within_period.filter(|d| ranges.iter().any(|range| range.contains(d)));

        // Ideally find an unused day
        let unused_days: Vec<i32> = possible_days
            .clone()
            .filter(|d| !used_days.contains(d))
            .collect();

        // If there are no unused days then it just chooses a random possible day
        let day = match unused_days.choose(rng) {
            Some(&d) => d,
            None => {
                let possible_days = possible_days.collect::<Vec<_>>();
                *possible_days.choose(rng).unwrap()
            }
        };

        self.days_until = day;
        used_days.insert(day);
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
            ..Default::default()
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
    pub fn from_task(task: &Task, do_date: NaiveDate) -> Self {
        DatedTask {
            id: task.id,
            do_date,
            instructions: task.instructions.clone(),
            days_until_when_added: task.days_until,
        }
    }
}
