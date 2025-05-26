use crate::notes::{DayNotes, NewNote, Note, ParsedDayNotes, ParsedNote};
use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use sqlx::{SqlitePool, migrate, prelude::FromRow};
pub async fn setup_db(fname: &str) -> NoteStore {
    let pool = SqlitePool::connect(fname).await.unwrap();
    migrate!().run(&pool).await.unwrap();
    return NoteStore { pool };
}
#[derive(FromRow)]
pub struct DateRow {
    id: u32,
    date: NaiveDate,
    task_count: u32,
    day_text: String,
}
#[derive(FromRow)]
pub struct NoteRow {
    pub id: u32,
    pub body: String,
    pub completed: bool,
    pub created_at: DateTime<Utc>,
    updated_at: Option<DateTime<Utc>>,
    deleted_at: Option<DateTime<Utc>>,
}
#[derive(FromRow)]
pub struct NoteRowDate {
    pub id: u32,
    pub body: String,
    pub completed: bool,
    pub created_at: DateTime<Utc>,
    updated_at: Option<DateTime<Utc>>,
    deleted_at: Option<DateTime<Utc>>,
    date: NaiveDate,
}

pub struct NoteStore {
    pub pool: SqlitePool,
}
impl NoteStore {
    pub async fn soft_delte_note_by_id(&self, id: u32) -> Result<()> {
        sqlx::query!(
            r#"UPDATE note SET deleted_at = (datetime('now')) WHERE id =?;"#,
            id
        )
        .execute(&self.pool)
        .await
        .context("Failed to soft delete note.")
        .map(|_| ())
    }
    pub async fn fetch_day(&self, d: NaiveDate) -> Result<Option<DateRow>> {
        sqlx::query_as!(
            DateRow,
            r#"SELECT id "id: u32", date, task_count "task_count: u32", day_text FROM day WHERE date = ?1;"#,
            d
        )
        .fetch_optional(&self.pool)
        .await
        .context("Failed fetchig day.")
    }
    pub async fn update_note(&self, n: &Note) -> Result<Note> {
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
            n.body,
            n.completed,
            n.id,
        ).fetch_one(&self.pool).await.context(format!("Failed updating note {}", n.id)).map(|r| Note::from(r))
    }
    pub async fn insert_day(
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
    pub async fn insert_note(&self, n: NewNote) -> Result<Note> {
        let utc_naive = n.created_at.date_naive();
        let day_key = sqlx::query_scalar!(r#"SELECT id FROM day WHERE date=?1;"#, utc_naive)
            .fetch_one(&self.pool)
            .await? as u32;
        let note = self
            ._insert_note(&n.body, n.created_at, n.completed, day_key)
            .await
            .map(|id| n.to_note(id));
        note
    }
    async fn _insert_note(
        &self,
        body: impl AsRef<str>,
        created_at: DateTime<Utc>,
        completed: bool,
        day_key: u32,
    ) -> Result<u32> {
        let body = body.as_ref();
        sqlx::query_scalar!(
            r#"INSERT INTO note (body, created_at, completed, day_key) VALUES (?1, ?2, ?3, ?4) RETURNING id "id: u32";"#,
            body,
            created_at,
            completed,
            day_key,
        )
        .fetch_one(&self.pool)
        .await
        .context("Failed adding note.")
    }
    pub async fn persist_parsed_day_note(&self, note: ParsedDayNotes) -> Result<DayNotes> {
        let mut tx = self
            .pool
            .begin()
            .await
            .context("Failed to start transaction.")?;
        let day_key = sqlx::query_scalar!(
            r#"INSERT INTO day (date, task_count, day_text) 
            VALUES (?1, ?2, ?3) 
            ON CONFLICT (date) 
            DO UPDATE SET date=?1, task_count=?2, day_text=?3 RETURNING id;"#,
            note.date,
            note.note_count,
            note.day_text,
        )
        .fetch_one(&mut *tx)
        .await
        .context("Failied upserting day note.")?;
        let mut notes = vec![];
        for n in note.notes {
            let note = match n {
                ParsedNote::NewNote(n) => self
                    ._insert_note(&n.body, n.created_at, n.completed, day_key as u32)
                    .await
                    .map(|id| n.to_note(id))?,
                ParsedNote::Note(n) => {
                    self.update_note(&n).await?;
                    n
                }
            };
            notes.push(note);
        }
        tx.commit().await?;
        let note_count = notes.len() as u32;
        Ok(DayNotes {
            notes,
            date: note.date,
            day_text: note.day_text,
            note_count,
        })
    }

    pub async fn update_day_text(&self, date: NaiveDate, day_text: impl AsRef<str>) -> Result<()> {
        let day_text = day_text.as_ref();
        sqlx::query!(
            "UPDATE day SET day_text = ?1 WHERE date = ?2;",
            day_text,
            date,
        )
        .execute(&self.pool)
        .await
        .map(|_| ())
        .context("Failed while updating day text.")
    }
    pub async fn get_day_notes_in_range(
        &self,
        start_day: NaiveDate,
        end_day: NaiveDate,
    ) -> Result<Vec<DayNotes>> {
        let jobbies = sqlx::query_as!(
            NoteRowDate,
            r#"SELECT
            n.id "id: u32",
            n.body,
            n.completed "completed: bool",
            n.created_at "created_at: DateTime<Utc>",
            n.updated_at "updated_at: DateTime<Utc>",
            n.deleted_at "deleted_at: DateTime<Utc>",
            d.date
            FROM note as n INNER JOIN day as d ON n.day_key = d.id WHERE d.date BETWEEN ?1 AND ?2 and n.deleted_at IS NULL
            GROUP BY d.date ORDER BY n.created_at;"#,
            start_day,
            end_day
        )
        .fetch_all(&self.pool)
        .await
        .context(format!("Failed fetching day notes between days {}:{}.", start_day, end_day))?;
        let mut current_notes = vec![];
        let mut notes = vec![];
        let mut current_day = None;
        for row in jobbies {
            if current_day.is_none() {
                current_day = Some(row.date);
            }
            if current_day.unwrap() != row.date {
                let text =
                    sqlx::query_scalar!("SELECT day_text from day WHERE date = ?;", current_day)
                        .fetch_optional(&self.pool)
                        .await
                        .context("Failed fetching day summary text.")?;
                let note_count = current_notes.len() as u32;
                notes.push(DayNotes {
                    notes: std::mem::take(&mut current_notes),
                    date: current_day.unwrap(),
                    note_count,
                    day_text: text.unwrap_or(String::new()),
                });
                current_day = Some(row.date)
            }
            current_notes.push(Note::from(row));
        }
        Ok(notes)
    }
    pub async fn get_days_notes(&self, day: NaiveDate) -> Result<DayNotes> {
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
            .fetch_optional(&self.pool)
            .await
            .context("Failed fetching day summary text.")?;
        Ok(DayNotes {
            notes: jobbies.into_iter().map(|n| Note::from(n)).collect(),
            note_count: task_count,
            date: day,
            day_text: text.unwrap_or(String::new()),
        })
    }
}
