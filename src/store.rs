use crate::notes::{DayNotes, NewNote, Note};
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
    pub async fn update_note(&self, n: Note) -> Result<Note> {
        self._update_note(n.id, n.body, n.completed)
            .await
            .map(Note::from)
    }
    pub async fn _update_note(
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
