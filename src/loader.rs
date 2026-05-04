use chrono::NaiveDate;
use serde::{de, Deserialize};
use std::error::Error;

use crate::task::AnnualWindow;

// Variants map to google sheets drop down options
#[derive(Copy, Clone, Debug, Default, Deserialize)]
pub enum Area {
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

// Variants map to google sheets drop down options
#[derive(Copy, Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub enum Season {
    #[default]
    Any,
    Summer,
}

impl Season {
    pub const SUMMER_WINDOW: AnnualWindow = AnnualWindow {
        start_month: 5,
        start_day: 1,
        end_month: 8,
        end_day: 31,
    };
}

/// Fields map to headers in google sheet
#[allow(dead_code)]
#[derive(Clone, Debug, Default, Deserialize)]
pub struct TaskRow {
    pub id: usize,
    #[serde(rename = "task")]
    pub instructions: String,
    pub period_days: i32,
    /// Current possible values: 1, 2, 3, 5, or 8
    pub effort: u8,
    pub area: Area,
    pub season: Season,
    pub created_at: NaiveDate,
}

#[allow(dead_code)]
pub mod effort {
    pub const EASY: u8 = 2;
    pub const MODERATE: u8 = 3;
    pub const HARD: u8 = 5;
    pub const EXTREME: u8 = 8;
}

pub fn get_sheet_url(sheet_id: &str, sheet_name: &str) -> String {
    format!(
        "https://docs.google.com/spreadsheets/d/{sheet_id}/gviz/tq?tqx=out:csv&sheet={sheet_name}"
    )
}

pub fn get_tasks(sheet_id: &str) -> Result<Vec<TaskRow>, Box<dyn Error>> {
    // Download the CSV data
    let sheet_url = get_sheet_url(sheet_id, "tasks");

    println!("Fetching tasks...");
    let response = reqwest::blocking::get(sheet_url)?;
    response.error_for_status_ref().unwrap();
    let csv = response.text()?;

    // Parse the CSV data
    let mut reader = csv::Reader::from_reader(csv.as_bytes());
    let mut tasks = Vec::new();

    for row in reader.deserialize::<TaskRow>() {
        match row {
            Ok(task) => {
                if tasks.iter().any(|t: &TaskRow| t.id == task.id) {
                    panic!("Found multiple tasks using ID {}", task.id);
                }
                if task.period_days == 0 {
                    println!(
                        "⚠️ Skipping task with period of 0 days, task ID = {}",
                        task.id
                    );
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
    println!("✅ Found {} task(s)", tasks.len());

    // Sort so that the output is consistent (with the same seed) regardless of how the sheet is sorted
    tasks.sort_by_key(|t| t.id);

    Ok(tasks)
}

pub fn get_holidays(sheet_id: &str, year: i32) -> Result<Vec<NaiveDate>, Box<dyn Error>> {
    // Download the CSV data
    let sheet_url = get_sheet_url(sheet_id, "holidays");

    println!("Fetching holidays...");
    let response = reqwest::blocking::get(sheet_url)?;
    response.error_for_status_ref().unwrap();
    let csv = response.text()?;

    // Parse the CSV data
    let mut reader = csv::Reader::from_reader(csv.as_bytes());
    let mut holidays = Vec::new();

    for row in reader.deserialize::<String>() {
        match row {
            Ok(date) => {
                let err_msg = "Expected date format MM-DD";
                let mut iter = date
                    .split_terminator('-')
                    .map(|s| s.parse::<u32>().expect(err_msg));

                let month = iter.next().expect(err_msg);
                let day = iter.next().expect(err_msg);
                assert!(iter.next().is_none(), "{}", err_msg);

                if let Some(date) = NaiveDate::from_ymd_opt(year, month, day) {
                    holidays.push(date);
                }
            }
            Err(e) => {
                eprintln!("Stopped parsing due to error: {}", e);
                break;
            }
        };
    }
    if holidays.is_empty() {
        println!("No holidays found");
    }
    println!("✅ Found {} holiday(s)", holidays.len());

    Ok(holidays)
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub struct TaskLog {
    pub id: usize,
    pub date: NaiveDate,
    #[serde(deserialize_with = "deserialize_google_bool")]
    pub done: bool,
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

pub fn get_history(sheet_id: &str) -> Result<Vec<TaskLog>, Box<dyn Error>> {
    // First we figure out what history to download by looking at sheet names. Each year should have
    // it's own sheet and they should be named "2025", "2026", etc. The `/edit` endpoint returns a ton
    // of data, but it is the only way I was able to dynamically extract sheet names. The alternative
    // is to manually provide the GID of each sheet, or to just check the headers/data which seems a bit finicky.
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

    let mut history = Vec::new();

    for year in years {
        // Setup url
        let sheet_url = get_sheet_url(sheet_id, year);

        // Download the data
        println!("Fetching history from {year}: {sheet_url}");
        let response = reqwest::blocking::get(sheet_url)?;
        response.error_for_status_ref().unwrap();
        let csv = response.text()?;

        // Parse the CSV data
        let mut reader = csv::Reader::from_reader(csv.as_bytes());

        for row in reader.deserialize::<TaskLog>() {
            match row {
                Ok(log) if log.done => {
                    history.push(log);
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

    println!("✅ Found {} completed task(s) in history", history.len());

    Ok(history)
}
