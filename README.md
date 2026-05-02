# SideraTask (WIP)

A simple CLI program that creates a schedule for recurring cleaning tasks.

### How it works

The program reads a list of possible tasks from a Google Sheets spreadsheet and then generates a schedule that tries to accomodate all tasks. The output is a .tsv file that can be copied directly into Google Sheets to keep track of completed tasks. After making a change to the task list or missing some scheduled tasks, the program can be ran again to integrate the new information and generate an updated schedule.

### Usage

1. Make a copy of [this template spreadsheet](https://docs.google.com/spreadsheets/d/1uoebxYatU9ud_ME0-J_4uzoEzqzodgb1DrWzt6Xblp8/edit?usp=sharing) in Google Sheets. The program relies on the spreadsheet using an exact format.
2. Customize the task list. Note that:
    1. Each task must have a unique ID.
    2. Currently only 1 task can be scheduled per day, so for best results the `period_days` across all tasks should ensure that the total average tasks per day rate is less than or equal to 1. If it is higher than 1 then the longer period tasks may be neglected. To check this, simply sum over the reciprocals of `period_days` column, for example using this formula `=SUMPRODUCT(IFERROR(1/C2:C, 0))`.
    3. The algorithm will not schedule high effort (5+) tasks on back to back days.
    4. `effort` is a drop down in the template but the value can be any number.
    4. Tasks with `season` set to `Summer` can only be scheduled between May 1st and August 31.
    5. Currently `season` and `area` options are limited to what is found in the template.
    5. Currently `area` and `created_at` are not used in the algorithm.
3. Customize the `holidays` tab. Holiday dates must follow MM-DD format. No tasks can be done on holidays.
4. Set the share access to "Anyone with link" so that the program can freely download the spreadsheet data (only viewing permission is required).
5. Get the sheet ID from the share url (for the template it would be `1uoebxYatU9ud_ME0-J_4uzoEzqzodgb1DrWzt6Xblp8`) and run the program with:
    ```bash
    # <SEED> must be a non-negative integer
    cargo run -- --sheet-id <SHEET_ID> --year <YEAR> [--seed <SEED>]
    # For help use:
    cargo run -- --help
    ```
    Note that currently the algorithm always generates up to Dec 31st of the given year.
6. Copy the contents of the output file found at `output/cleaning-list.tsv` and paste it into a new tab named "\<YEAR\>" (e.g. "2026") in the same spreadsheet. The column headers must be named `ID`, `DATE`, `DONE`, `TASK`. Insert checkboxes into the `DONE` column. Refer to the template for an example.
7. If you make changes to the main task list or if you miss some scheduled tasks, you may want to regenerate the schedule. To do this, make sure you have checked off all the completed tasks in the history tab, and then simply re-run the program. The program will take all of your history into account when creating the schedule. The start date will automatically be set to the day after the most recently completed task.

---

WIP
