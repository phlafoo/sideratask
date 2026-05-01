// #![allow(unused)]

use chrono::NaiveDate;
use clap::Parser;
use generator::generate_cleaning_list;
use serde::{de, Deserialize};
use std::error::Error;
use std::fs::File;
use std::io::Write;
use std::path::Path;

use crate::generator::DatedTask;

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
pub struct TaskRow {
    id: usize,
    #[serde(rename = "task")]
    instructions: String,
    period_days: i32,
    /// Current possible values: 1, 2, 3, 5, or 8
    effort: u8,
    area: Area,
    season: Season,
    created_at: NaiveDate,
}

#[allow(dead_code)]
mod effort {
    pub const EASY: u8 = 2;
    pub const MODERATE: u8 = 3;
    pub const HARD: u8 = 5;
    pub const EXTREME: u8 = 8;
}

#[derive(Parser, Debug)]
struct Args {
    /// Which year to generate the cleaning list for
    #[arg(short, long)]
    year: i32,
    #[arg(long)]
    /// Can be found in sheet share url: https://docs.google.com/spreadsheets/d/<sheet_id>/edit?usp=sharing
    sheet_id: String,
    /// (Optional) Seed for RNG. Must be a positive number.
    #[arg(short, long)]
    seed: Option<u64>,
}

// cargo run -- --help
fn main() -> Result<(), Box<dyn Error>> {
    let Args {
        year,
        sheet_id,
        seed,
    } = Args::parse();

    let tasks = get_tasks(&sheet_id)?;
    let history = get_history(&sheet_id)?;

    println!("Generating cleaning list...");
    let cleaning_list = generate_cleaning_list(&tasks, history, year, seed);
    println!("Finished generating cleaning list!");

    print_summary(&tasks, &cleaning_list);

    // Create TSV string (an extra tab is placed after date column for the checkbox column)
    let tsv = cleaning_list
        .iter()
        .map(|task| format!("{}\t{}\t\t{}", task.id, task.do_date, task.instructions))
        .collect::<Vec<String>>()
        .join("\n");

    // DEBUG
    // let tsv = cleaning_list
    //     .iter()
    //     .map(|task| {
    //         format!(
    //             "{}\t{}\t{}\t{}",
    //             task.id, task.do_date, task.days_until_when_added, task.instructions
    //         )
    //     })
    //     .collect::<Vec<String>>()
    //     .join("\n")

    let path = Path::new("output/cleaning-list.tsv");
    let mut tsv_file = File::create(path)?;
    tsv_file.write_all(tsv.as_bytes())?;

    println!("✅ Success! Saved to file: {:#?}", path);

    Ok(())
}

fn get_sheet_url(sheet_id: &str, sheet_name: &str) -> String {
    format!(
        "https://docs.google.com/spreadsheets/d/{sheet_id}/gviz/tq?tqx=out:csv&sheet={sheet_name}"
    )
}

fn get_tasks(sheet_id: &str) -> Result<Vec<TaskRow>, Box<dyn Error>> {
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

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
struct TaskLog {
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

fn get_history(sheet_id: &str) -> Result<Vec<TaskLog>, Box<dyn Error>> {
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

fn print_summary(tasks: &[TaskRow], cleaning_list: &[DatedTask]) {
    println!("Summary:");
    println!("period | count | task");
    println!("-------+-------+-----");
    for task in tasks.iter() {
        let count = cleaning_list.iter().filter(|dt| dt.id == task.id).count();
        println!(
            "{:>6} | {:>5} | {}",
            task.period_days, count, task.instructions
        );
    }
    println!("-------+-------+-----");
}
