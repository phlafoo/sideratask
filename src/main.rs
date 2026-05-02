// #![allow(unused)]

use clap::Parser;
use generator::generate_cleaning_list;
use std::error::Error;
use std::fs::File;
use std::io::Write;
use std::path::Path;

use crate::generator::DatedTask;
use crate::loader::{get_history, get_holidays, get_tasks, TaskRow};

mod generator;
mod loader;

#[derive(Parser, Debug)]
struct Args {
    /// Which year to generate the cleaning list for
    #[arg(short, long)]
    year: i32,
    #[arg(long)]
    /// Can be found in sheet share url: https://docs.google.com/spreadsheets/d/<sheet_id>/edit?usp=sharing
    sheet_id: String,
    /// (Optional) Seed for RNG. Must be a non-negative integer.
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
    let holidays = get_holidays(&sheet_id, year)?;
    let history = get_history(&sheet_id)?;

    println!("Generating cleaning list...");
    let cleaning_list = generate_cleaning_list(&tasks, &holidays, history, year, seed);
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
