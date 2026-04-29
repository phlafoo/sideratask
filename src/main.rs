// TODO remove
#![allow(unused)]

use chrono::{Datelike, Duration, NaiveDate};
use clap::Parser;
use fnv::FnvHashMap;
use generator::generate_cleaning_list;
use serde::{de, Deserialize};
use std::cmp::Reverse;
use std::env;
use std::error::Error;
use std::fs::File;
use std::io::Write;
use std::path::Path;

mod generator;

// Variants map to google sheets drop down variants
#[derive(Copy, Clone, Debug, Default, Deserialize)]
enum Area {
    Multi,
    Bathroom,
    Bedroom,
    Deck,
    Kitchen,
    #[serde(rename = "Laundry room")]
    Laundry,
    Entryway,
    Loft,
    #[serde(rename = "Living area")]
    Living,
    /// Used for "day off" task
    #[serde(skip)]
    #[default]
    None,
}

// Variants map to google sheets drop down variants
#[derive(Copy, Clone, Debug, Default, Deserialize, Eq, PartialEq)]
enum Season {
    #[default]
    Any,
    Summer,
}

/// Fields map to headers in google sheet
#[allow(dead_code)]
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Task {
    id: usize,
    #[serde(rename = "task")]
    instructions: String,
    period_days: i32,
    effort: u8, // 1, 2, 3, 5, or 8
    area: Area,
    season: Season,
    created_at: NaiveDate,
    /// How many days until this task should be done again. Negative value mean it is overdue.
    #[serde(skip)]
    days_until: i32,
}

impl Task {
    const DAY_OFF_ID: usize = 0;

    fn get_day_off_task(period_days: i32) -> Self {
        Task {
            id: Task::DAY_OFF_ID,
            instructions: "Day off".to_string(),
            period_days,
            effort: 0,
            area: Area::None,
            season: Season::Any,
            created_at: NaiveDate::MIN,
            days_until: 0,
        }
    }

    fn get_holiday_task() -> Self {
        let mut task = Task::get_day_off_task(0);
        task.instructions = "Holiday".to_string();
        task
    }
}

#[derive(Parser, Debug)]
struct Args {
    #[arg(short, long)]
    year: i32,
    #[arg(short, long)]
    sheet_id: String,
}

/// Run with `cargo run -- --year <year> --sheet-id <sheet_id>`
fn main() -> Result<(), Box<dyn Error>> {
    // sheet_id comes from public share link for google sheet
    let Args { year, sheet_id } = Args::parse();

    let mut tasks = get_tasks(&sheet_id)?;
    let start_date = process_history(&mut tasks, &sheet_id, year)?;
    let tsv = generate_cleaning_list(tasks, start_date);

    let path = Path::new("output/cleaning-list.tsv");
    let mut tsv_file = File::create(path)?;
    tsv_file.write_all(tsv.as_bytes())?;

    println!("Saved to file: {:#?}", path);

    Ok(())
}

fn get_sheet_url(sheet_id: &str, sheet_name: &str) -> String {
    format!(
        "https://docs.google.com/spreadsheets/d/{sheet_id}/gviz/tq?tqx=out:csv&sheet={sheet_name}"
    )
}

fn get_tasks(sheet_id: &str) -> Result<Vec<Task>, Box<dyn Error>> {
    // Setup url
    let sheet_url = get_sheet_url(sheet_id, "tasks");

    // Download the data
    println!("Fetching data...");
    let response = reqwest::blocking::get(sheet_url)?;
    response.error_for_status_ref().unwrap();
    let csv = response.text()?;

    // Parse the CSV data
    let mut reader = csv::Reader::from_reader(csv.as_bytes());
    let mut tasks = Vec::new();

    for row in reader.deserialize::<Task>() {
        match row {
            Ok(task) => {
                if task.id == Task::DAY_OFF_ID {
                    panic!("Task ID 0 is reserved for implicit 'day off' task");
                }
                if task.period_days == 0 {
                    println!("Skipping task with period of 0 days, task ID = {}", task.id);
                    continue;
                }
                tasks.push(task);
            }
            Err(e) => {
                eprintln!("Stopped parsing due to error: {}", e);
                break;
            }
        };
    }
    if tasks.is_empty() {
        panic!("No tasks found");
    }
    println!("Fetched {} tasks", tasks.len());

    // Sort so that the output is consistent (with the same seed) regardless of how the sheet is sorted
    tasks.sort_by_key(|t| t.id);

    Ok(tasks)
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
struct TaskRecord {
    id: usize,
    date: NaiveDate,
    #[serde(deserialize_with = "deserialize_google_bool")]
    done: bool,
}

/// "TRUE" => true, "FALSE" or "" => false
fn deserialize_google_bool<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;

    match s.to_uppercase().as_str() {
        "TRUE" => Ok(true),
        "FALSE" | "" => Ok(false),
        _ => Err(de::Error::invalid_value(
            de::Unexpected::Str(&s),
            &"a boolean string (\"TRUE\", \"FALSE\", or \"\")",
        )),
    }
}

/// Returns start date (day after most recent history)
fn process_history(
    tasks: &mut [Task],
    sheet_id: &str,
    current_year: i32,
) -> Result<NaiveDate, Box<dyn Error>> {
    // First we get history by looking at sheet names. Each year should have it's own sheet and they
    // should be named "2025", "2026", etc. The `/edit` endpoint returns a ton of data, but it is the
    // only way I was able to dynamically extract sheet names. The alternative is to manually provide
    // the GID of each sheet, or to just check the headers/data which seems a bit finicky.
    let url = format!("https://docs.google.com/spreadsheets/d/{sheet_id}/edit");
    let response = reqwest::blocking::get(url)?;
    let html = response.text()?;

    // Hopefully google doesn't change this css class lol
    let sheet_name_years_regex =
        regex::Regex::new(r#"goog-inline-block docs-sheet-tab-caption\">(?<sheet_name>\d{4})<"#)?;

    let years: Vec<&str> = sheet_name_years_regex
        .captures_iter(&html)
        .filter_map(|c| c.name("sheet_name").map(|s| s.as_str()))
        .collect();

    let mut records = Vec::new();

    for year in years {
        // Setup url
        let sheet_url = get_sheet_url(sheet_id, year);

        // Download the data
        println!("Fetching history from {year}: {sheet_url}");
        let response = reqwest::blocking::get(sheet_url)?;
        let csv = response.text()?;

        // Parse the CSV data
        let mut reader = csv::Reader::from_reader(csv.as_bytes());

        for row in reader.deserialize::<TaskRecord>() {
            match row {
                Ok(task_record) if task_record.done => {
                    records.push(task_record);
                }
                Err(e) => {
                    // If the sheet has any junk below the last row of data, this will trigger, so it
                    // isn't necessarily a problem.
                    eprintln!("Stopped parsing due to error: {}", e);
                    break;
                }
                _ => {}
            };
        }
    }

    println!("Found {} completed tasks in history", records.len());

    // The start date will be the day after the most recent record
    records.sort_by_key(|r| Reverse(r.date));

    let Some(most_recent_record) = records.first() else {
        // No history, default to Jan 1st
        return Ok(NaiveDate::from_ymd_opt(current_year, 1, 1).unwrap());
    };

    let start_date = most_recent_record.date + Duration::days(1);
    if start_date.year() > current_year {
        panic!("History is in the future or too recent");
    }

    // All that is needed to integrate history into the algorithm, is to set the days_until based on
    // the last time the task was done
    for task in tasks.iter_mut() {
        // Scan records in descending order of date to get the most recent one
        if let Some(task_record) = records.iter().find(|r| r.id == task.id) {
            let days_ago = (start_date - task_record.date).num_days() as i32;
            task.days_until = task.period_days - days_ago;
        }
    }

    // for task_record in &records {
    //     let task = tasks
    //         .iter_mut()
    //         .find(|t| t.id == task_record.id)
    //         .unwrap_or_else(|| panic!("History contains invalid task with ID {}", task_record.id));
    //     let days_ago = (start_date - task_record.date).num_days() as i32;
    //     task.days_until = task.period_days - days_ago;
    // }

    Ok(start_date)
}
