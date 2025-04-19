use std::{
    fs::File,
    io::{Read, Seek, Write},
    process,
    str::FromStr,
};

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, NaiveDate, Utc};
use clap::Parser;
use env_logger::Env;
use log::info;
use sqlx::{SqlitePool, prelude::FromRow};
use tempfile::NamedTempFile;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Mode::parse();
    let store = setup_db().await;
    env_logger::init_from_env(Env::new().default_filter_or("info"));

    match args {
        Mode::New { note_body } => {
            let note = NewNote::new(note_body);
            store.insert_note(note).await.unwrap();
        }
        Mode::Edit => {
            let editor = std::env::var("EDITOR").unwrap_or(String::from("vim"));
            let notes = store.get_days_notes(Utc::now().date_naive()).await.unwrap();
            let mut file = NamedTempFile::new()?;
            // Try happy path on failure clean the file.
            file.write(notes.pretty().as_bytes())?;
            process::Command::new(editor).arg(file.path()).status()?;
            let mut new_notes = String::new();
            file.seek(std::io::SeekFrom::Start(0))?;
            file.read_to_string(&mut new_notes)?;
            let notes = parse_notes_string(new_notes, &store).await?;
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
        date = line.strip_prefix("Today: ");
    }
    let date = date.ok_or(anyhow!("Couldn't find text."))?;
    let day = NaiveDate::from_str(date)?;
    let day_notes = store.get_days_notes(day).await?;
    let mut notes = vec![];
    let day_note_ids = day_notes.notes.iter().map(|n| n.id).collect::<Vec<u32>>();
    let mut seen_notes = Vec::with_capacity(day_note_ids.len());
    // Update notes that have common ids.
    for line in line_iter {
        let line = line.trim();
        let Some(n) = Note::from_pretty(store, line).await? else {
            continue;
        };
        seen_notes.push(n.id);
        notes.push(n);
    }
    // Delete notes that have been removed.
    for note_id in day_note_ids {
        if !seen_notes.contains(&note_id) {
            store.soft_delte_note_by_id(note_id).await?;
        }
    }
    notes.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    store.get_days_notes(day).await
}

async fn setup_db() -> NoteStore {
    let pool = SqlitePool::connect("sqlite://db.db").await.unwrap();
    return NoteStore { pool };
}
/// Mode enum descibes state that the program runs in, write or read mode.
#[derive(Parser, Debug)]
enum Mode {
    Check,
    Edit,
    New { note_body: String },
    Show,
}

#[derive(Debug)]
struct Note {
    id: u32,
    body: String,
    completed: bool,
    created_at: DateTime<Utc>,
}
impl From<NoteRow> for Note {
    fn from(value: NoteRow) -> Self {
        Note {
            id: value.id,
            body: value.body,
            completed: value.completed,
            created_at: value.created_at,
        }
    }
}
impl Note {
    fn date_created(&self) -> NaiveDate {
        self.created_at.date_naive()
    }
    fn pretty(&self) -> String {
        let tick = if self.completed { "x" } else { " " };
        format!(" - [{tick}] :{}: {}", self.id, self.body)
    }
    async fn from_pretty(store: &NoteStore, s: impl AsRef<str>) -> Result<Option<Note>> {
        let s = s.as_ref();
        let s = s.trim();
        if &s[..7] != "- [ ] :" || &s[..7] != "- [x] :" {
            return Err(anyhow!("Invalid note start."));
        }
        let tick_char = s.chars().nth(4).ok_or(anyhow!(
            "Invalid format for note string expect 5th char to be tick box."
        ))?;
        let completed = tick_char == 'x';
        let idx = s
            .find(":")
            .ok_or(anyhow!("Malformed note string expect :"))?;
        match s[idx + 1..].split_once(':') {
            Some((id_string, text)) => {
                let body = String::from(text.trim());
                let id = id_string.parse::<u32>().context(format!(
                    "Parsing {} failed. {}",
                    id_string,
                    &s[idx + 1..]
                ))?;
                return store
                    ._update_note(id, body, completed)
                    .await
                    .map(Note::from)
                    .map(Some);
            }
            None => {
                let new_note_text = s[idx + 1..].trim();
                if new_note_text.is_empty() {
                    return Ok(None);
                }
                return store
                    .insert_note(NewNote {
                        body: String::from(new_note_text),
                        completed,
                        created_at: Utc::now(),
                    })
                    .await
                    .map(Some);
            }
        };
    }
}
struct NewNote {
    body: String,
    completed: bool,
    created_at: DateTime<Utc>,
}
impl NewNote {
    fn date_created(&self) -> NaiveDate {
        self.created_at.date_naive()
    }
    fn to_note(self, id: u32) -> Note {
        Note {
            id,
            body: self.body,
            completed: self.completed,
            created_at: self.created_at,
        }
    }
    fn new(body: impl Into<String>) -> NewNote {
        NewNote {
            body: body.into(),
            completed: false,
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug)]
struct DayNotes {
    notes: Vec<Note>,
    note_count: u32,
    date: NaiveDate,
    day_text: String,
}
impl DayNotes {
    fn pretty(&self) -> String {
        let mut out = format!("Today: {}\n", self.date.to_string());
        for note in &self.notes {
            out.push_str(&format!("{}\n", note.pretty()));
        }
        out
    }
}
#[derive(FromRow)]
struct DateRow {
    id: u32,
    date: NaiveDate,
    task_count: u32,
    day_text: String,
}
#[derive(FromRow)]
struct NoteRow {
    id: u32,
    body: String,
    completed: bool,
    created_at: DateTime<Utc>,
    updated_at: Option<DateTime<Utc>>,
    deleted_at: Option<DateTime<Utc>>,
}

struct NoteStore {
    pool: SqlitePool,
}
impl NoteStore {
    async fn soft_delte_note_by_id(&self, id: u32) -> Result<()> {
        sqlx::query!(
            r#"UPDATE note SET deleted_at = (datetime('now')) WHERE id =?;"#,
            id
        )
        .execute(&self.pool)
        .await
        .context("Failed to soft delete note.")
        .map(|_| ())
    }
    async fn fetch_day(&self, d: NaiveDate) -> Result<Option<DateRow>> {
        sqlx::query_as!(
            DateRow,
            r#"SELECT id "id: u32", date, task_count "task_count: u32", day_text FROM day WHERE date = ?1;"#,
            d
        )
        .fetch_optional(&self.pool)
        .await
        .context("Failed fetchig day.")
    }
    async fn update_note(&self, n: Note) -> Result<Note> {
        self._update_note(n.id, n.body, n.completed)
            .await
            .map(Note::from)
    }
    async fn _update_note(
        &self,
        id: u32,
        body_text: impl AsRef<str>,
        completed: bool,
    ) -> Result<NoteRow> {
        let body_text = body_text.as_ref();
        sqlx::query_as!(
            NoteRow,
            r#"UPDATE  note SET body = ?1, completed = ?2, updated_at = (datetime('now')) WHERE id = ?3
            RETURNING id "id: u32",
            body,
            completed "completed: bool",
            created_at "created_at: DateTime<Utc>",
            updated_at "updated_at: DateTime<Utc>",
            deleted_at "deleted_at: DateTime<Utc>"
            "#,
            body_text,
            completed,
            id,
        ).fetch_one(&self.pool).await.context(format!("Failed updating note {}", id))
    }
    async fn insert_day(
        &self,
        d: NaiveDate,
        task_count: Option<u32>,
        text: impl AsRef<str>,
    ) -> Result<DateRow> {
        let task_count = task_count.unwrap_or(0) as i64;
        let text = text.as_ref();
        sqlx::query_as!(
            DateRow,
            r#"INSERT INTO day (date, task_count, day_text) VALUES (?1, ?2, ?3) RETURNING id "id: u32", date, task_count "task_count:u32", day_text;"#,
            d,
            task_count,
            text
        ).fetch_one(&self.pool).await.context("Failed inserting day.")
    }
    async fn insert_note(&self, n: NewNote) -> Result<Note> {
        let date = self
            .fetch_day(n.date_created())
            .await
            .context("Failed day query before adding note.")?;

        let d = match date {
            Some(d) => d,
            None => self
                .insert_day(n.date_created(), None, "")
                .await
                .context("Failed making new day on first note addition.")?,
        };
        let iso_time = n.created_at.to_rfc3339();
        sqlx::query_scalar!(
            r#"INSERT INTO note (body, created_at, completed, day_key) VALUES (?1, ?2, ?3, ?4) RETURNING id "id: u32";"#,
            n.body,
            iso_time,
            n.completed,
            d.id
        )
        .fetch_one(&self.pool)
        .await
        .context("Failed adding note.").map(|id| n.to_note(id))
    }

    async fn get_days_notes(&self, day: NaiveDate) -> Result<DayNotes> {
        let jobbies = sqlx::query_as!(
            NoteRow,
            r#"SELECT
            n.id "id: u32",
            n.body,
            n.completed "completed: bool",
            n.created_at "created_at: DateTime<Utc>",
            n.updated_at "updated_at: DateTime<Utc>",
            n.deleted_at "deleted_at: DateTime<Utc>"
            FROM note as n INNER JOIN day as d ON n.day_key = d.id WHERE d.date =? and n.deleted_at IS NULL
            ORDER BY n.created_at;"#,
            day
        )
        .fetch_all(&self.pool)
        .await
        .context("Failed fetching day notes.")?;
        let task_count = jobbies.len() as u32;
        let text = sqlx::query_scalar!("SELECT day_text from day WHERE date = ?;", day)
            .fetch_one(&self.pool)
            .await
            .context("Failed fetching day summary text.")?;
        Ok(DayNotes {
            notes: jobbies.into_iter().map(|n| Note::from(n)).collect(),
            note_count: task_count,
            date: day,
            day_text: text,
        })
    }
}
