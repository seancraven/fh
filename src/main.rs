pub mod notes;
pub mod store;
use std::{
    fs::File,
    io::{Read, Seek, Write},
    path::PathBuf,
    process,
    str::FromStr,
};

use crate::store::setup_db;
use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Days, Local, NaiveDate, TimeZone};
use clap::{Parser, Subcommand};
use env_logger::Env;
use log::{debug, info};
use notes::{DayNotes, NewNote, Note};
use store::NoteStore;
use tempfile::NamedTempFile;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Mode::parse();
    let home = std::env::var("HOME")?;
    // Setup fuckhead config.
    let db_path = PathBuf::from(home).join(".fuckhead/db.db");
    let parent = db_path.parent().unwrap();
    if !parent.exists() {
        debug!("Creating parent config dir at {}", parent.display());
        std::fs::create_dir(parent).unwrap();
    }
    if !db_path.exists() {
        File::create(&db_path)?;
    }
    let store = setup_db(&format!("sqlite:///{}", &db_path.to_str().unwrap())).await;
    env_logger::init_from_env(Env::new().default_filter_or("critical"));

    match args {
        Mode::New { note_body } => {
            let note = NewNote::new(note_body);
            store.insert_note(note).await.unwrap();
        }
        Mode::Edit { day } => {
            edit(&store, day).await?;
            show(&store, day).await?;
        }
        Mode::Check => {
            let day = Local::now().date_naive();
            let notes = store.get_days_notes(day).await?;
            if notes.note_count == 0 {
                edit(&store, None).await?
            } else {
                show(&store, None).await?
            }
        }
        Mode::Show { day, period } => match period {
            None => show(&store, day).await?,
            Some(p) => show_range(&store, day, p.to_day_count()).await?,
        },
    }
    Ok(())
}
fn map_day<Tz>(start_datetime: DateTime<Tz>, day: Option<i32>) -> NaiveDate
where
    Tz: TimeZone,
{
    let Some(day) = day else {
        return start_datetime.naive_utc().date();
    };
    let target_datetime;
    if day > 0 {
        target_datetime = start_datetime
            .checked_add_days(Days::new(day as u64))
            .expect("Don't account for leap");
    } else {
        target_datetime = start_datetime
            .checked_sub_days(Days::new(day.abs() as u64))
            .expect("Don't account for leap");
    }
    target_datetime.naive_utc().date()
}

/// Run the edit subcommand open the prefered editor (should be vim)
/// get the daily notes and update any changes made by the user.
async fn edit(store: &NoteStore, day: Option<i32>) -> Result<()> {
    let editor = std::env::var("EDITOR").unwrap_or(String::from("vim"));
    let target_day = map_day(Local::now(), day);
    let notes = store.get_days_notes(target_day).await.unwrap();
    let mut file = NamedTempFile::with_suffix(".md")?;
    // Try happy path on failure clean the file.
    file.write_all(notes.pretty_md().as_bytes())?;
    process::Command::new(editor).arg(file.path()).status()?;
    let mut new_notes = String::new();
    file.seek(std::io::SeekFrom::Start(0))?;
    file.read_to_string(&mut new_notes)?;
    parse_notes_string(new_notes, &store).await?;
    Ok(())
}

async fn show_range(store: &NoteStore, day: Option<i32>, time_span: usize) -> Result<()> {
    let day = day.unwrap_or(0);
    let start_day = map_day(Local::now(), Some(-(time_span as i32) + day));
    let end_day = map_day(Local::now(), Some(time_span as i32 + day));
    let all_notes = store
        .get_day_notes_in_range(start_day, end_day)
        .await
        .context("Failed querying all notes.")?;
    let mut out = String::new();
    for note in all_notes {
        out.push_str(&note.pretty())
    }
    println!("{}", out);
    Ok(())
}
/// Run show sucommand, print current state to terminal.
async fn show(store: &NoteStore, day: Option<i32>) -> Result<()> {
    let target_day = map_day(Local::now(), day);

    let notes = store.get_days_notes(target_day).await?;
    info!(
        "found {} notes for {}",
        notes.note_count,
        notes.date.to_string()
    );
    println!("{}", notes.pretty());
    Ok(())
}

/// Compare the current database state to that input by the user, perform the inserts and soft deltes required to
/// maintain the state between the frontend (notes) and db.
async fn parse_notes_string(s: String, store: &NoteStore) -> Result<DayNotes> {
    let mut line_iter = s.lines();
    let mut date: Option<&str> = None;
    while date.is_none() {
        let Some(line) = line_iter.next() else {
            return Err(anyhow!("Couldn't find text."));
        };
        if line.trim().is_empty() {
            continue;
        }
        date = line.strip_prefix("# Today: ");
        if date.is_none() {
            date = line.strip_prefix("# Day: ")
        }
    }
    let date = date.ok_or(anyhow!("Couldn't find text."))?;
    let day = NaiveDate::from_str(date)?;
    let mut day_notes = store.get_days_notes(day).await?;
    let day_note_ids = day_notes.notes.iter().map(|n| n.id).collect::<Vec<u32>>();
    let mut seen_notes = Vec::with_capacity(day_note_ids.len());
    let mut free_text = String::new();
    // Update notes by line.
    for line in line_iter {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match line.chars().next().unwrap() {
            '-' => {
                let Some(n) = Note::from_pretty(store, line).await? else {
                    continue;
                };
                seen_notes.push(n.id);
            }
            _ => {
                free_text.push_str(line);
                free_text.push_str("\n");
            }
        }
    }
    if !free_text.is_empty() && free_text != day_notes.day_text {
        day_notes.day_text = free_text;
        store
            .update_day_text(day_notes.date, &day_notes.day_text)
            .await?;
    }
    // Delete notes that have been removed.
    for note_id in day_note_ids {
        if !seen_notes.contains(&note_id) {
            store.soft_delte_note_by_id(note_id).await?;
        }
    }
    store.get_days_notes(day).await
}

#[derive(Subcommand, Debug)]
enum Period {
    Week,
    Month,
}
impl Period {
    fn to_day_count(&self) -> usize {
        match *self {
            Self::Week => 7,
            Self::Month => 30,
        }
    }
}
/// Mode enum descibes state that the program runs in, write or read mode.
#[derive(Parser, Debug)]
enum Mode {
    /// Check if new notes need to be added.
    Check,
    /// Edit current day's notes.
    ///
    Edit {
        #[arg(short, long, default_value=None, allow_hyphen_values=true)]
        day: Option<i32>,
    },
    /// Make a new note.
    New { note_body: String },
    /// Show current day's notes.
    Show {
        #[arg(short, long, default_value=None, allow_hyphen_values=true)]
        day: Option<i32>,
        #[command(subcommand)]
        period: Option<Period>,
    },
}

#[cfg(test)]
mod tests {
    use crate::map_day;
    use chrono::{Days, Local, Timelike};

    #[test]
    fn test_date() {
        let time = Local::now();
        for hour in 0..24 {
            let target_time = time.with_hour(hour).unwrap();
            for day in 0..2 {
                let out = map_day(target_time, Some(day));
                let out_base = target_time
                    .checked_add_days(Days::new(day as u64))
                    .unwrap()
                    .naive_utc()
                    .date();
                assert_eq!(out, out_base);
            }
        }
    }
}
