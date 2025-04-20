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
use anyhow::{Result, anyhow};
use chrono::{NaiveDate, Utc};
use clap::Parser;
use env_logger::Env;
use log::{debug, info};
use notes::{DayNotes, NewNote, Note};
use store::NoteStore;
use tempfile::NamedTempFile;
#[tokio::main]
async fn main() -> Result<()> {
    let args = Mode::parse();
    let home = std::env::var("HOME")?;
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
        Mode::Edit => {
            let editor = std::env::var("EDITOR").unwrap_or(String::from("vim"));
            let notes = store.get_days_notes(Utc::now().date_naive()).await.unwrap();
            let mut file = NamedTempFile::with_suffix(".md")?;
            // Try happy path on failure clean the file.
            file.write(notes.pretty_md().as_bytes())?;
            process::Command::new(editor).arg(file.path()).status()?;
            let mut new_notes = String::new();
            file.seek(std::io::SeekFrom::Start(0))?;
            file.read_to_string(&mut new_notes)?;
            parse_notes_string(new_notes, &store).await?;
        }
        Mode::Check => todo!(),
        Mode::Show => {
            let notes = store.get_days_notes(Utc::now().date_naive()).await?;
            info!(
                "found {} notes for {}",
                notes.note_count,
                notes.date.to_string()
            );
            println!("{}", notes.pretty());
        }
    }
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

/// Mode enum descibes state that the program runs in, write or read mode.
#[derive(Parser, Debug)]
enum Mode {
    Check,
    Edit,
    New { note_body: String },
    Show,
}
