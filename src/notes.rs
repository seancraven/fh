use crate::store::{NoteRow, NoteRowDate, NoteStore};
use ansi_term::{Color, Style};
use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, NaiveDate, Utc};

#[derive(Debug)]
pub struct Note {
    pub id: u32,
    pub body: String,
    pub completed: bool,
    pub created_at: DateTime<Utc>,
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
impl From<NoteRowDate> for Note {
    fn from(value: NoteRowDate) -> Self {
        Note {
            id: value.id,
            body: value.body,
            completed: value.completed,
            created_at: value.created_at,
        }
    }
}
impl Note {
    pub fn date_created(&self) -> NaiveDate {
        self.created_at.date_naive()
    }
    pub fn pretty_empty() -> String {
        String::from(" - [ ] :")
    }
    pub fn pretty(&self) -> String {
        let tick = if self.completed { "x" } else { " " };
        format!(" - [{tick}] :{}: {}", self.id, self.body)
    }
    /// Insert and build note from string.
    pub async fn from_pretty(store: &NoteStore, s: impl AsRef<str>) -> Result<Option<Note>> {
        let s = s.as_ref();
        let s = s.trim();
        if !(&s[..7] == "- [ ] :" || &s[..7] == "- [x] :") {
            return Err(anyhow!("Invalid note start. {}", &s[..7]));
        }
        let tick_char = s.chars().nth(3).ok_or(anyhow!(
            "Invalid format for note string expect 3th char to be tick box."
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
pub struct NewNote {
    pub body: String,
    pub completed: bool,
    pub created_at: DateTime<Utc>,
}
impl NewNote {
    pub fn date_created(&self) -> NaiveDate {
        self.created_at.date_naive()
    }
    pub fn to_note(self, id: u32) -> Note {
        Note {
            id,
            body: self.body,
            completed: self.completed,
            created_at: self.created_at,
        }
    }
    pub fn new(body: impl Into<String>) -> NewNote {
        NewNote {
            body: body.into(),
            completed: false,
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug)]
pub struct DayNotes {
    pub notes: Vec<Note>,
    pub note_count: u32,
    pub date: NaiveDate,
    pub day_text: String,
}
impl DayNotes {
    pub fn day_prefix(&self) -> &'static str {
        if self.date == Utc::now().date_naive() {
            "Today"
        } else {
            "Day"
        }
    }
    pub fn pretty_md(&self) -> String {
        let mut out = format!("# {}: {}\n\n", self.day_prefix(), self.date);
        for note in &self.notes {
            out.push_str(&format!("{}\n", note.pretty()));
        }
        out.push_str(&format!("{}\n", Note::pretty_empty()));
        out.push('\n');
        out.push_str(&self.day_text);
        out
    }
    pub fn pretty(&self) -> String {
        let mut out = format!(
            "{}: {} \n\n",
            self.day_prefix(),
            Color::Green.paint(self.date.to_string())
        );
        out = Style::new().bold().paint(out).to_string();
        for note in &self.notes {
            out.push_str(&format!("{}\n", note.pretty()));
        }
        if self.notes.is_empty() {
            out.push_str("No Notes.");
        }
        out.push('\n');
        out.push_str(&self.day_text);
        out
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        notes::{NewNote, Note},
        store::setup_db,
    };
    use sqlx::migrate;

    async fn setup_sqlitedb() -> crate::store::NoteStore {
        let s = setup_db("sqlite://:memory:").await;
        migrate!().run(&s.pool).await.unwrap();
        s
    }
    #[tokio::test]
    async fn test_parse_note() {
        let store = setup_sqlitedb().await;
        let n = Note::from_pretty(&store, "- [ ] : test")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(n.body, "test");
    }
    #[tokio::test]
    async fn test_parse_note_none() {
        let store = setup_sqlitedb().await;
        let n = Note::from_pretty(&store, "- [ ] :    ").await.unwrap();
        assert!(n.is_none());
    }
    #[tokio::test]
    async fn test_parse_note_not_exist() {
        let store = setup_sqlitedb().await;
        let n = Note::from_pretty(&store, "- [x] :10: hi").await;
        assert!(n.is_err())
    }
    #[tokio::test]
    async fn test_parse_note_exist() {
        let store = setup_sqlitedb().await;
        let n_base = store.insert_note(NewNote::new("test")).await.unwrap();
        let n = Note::from_pretty(&store, "- [x] :1: hi")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(n.body, "hi", "Expect body to update.");
        assert_eq!(n.id, n_base.id);
        assert_eq!(n.created_at, n_base.created_at);
        assert!(n.completed)
    }
    #[tokio::test]
    async fn test_parse_dirty() {
        let store = setup_sqlitedb().await;
        store.insert_note(NewNote::new("test")).await.unwrap();
        let n = Note::from_pretty(&store, "text\n- [x] :1: hi").await;
        assert!(n.is_err())
    }
    #[tokio::test]
    async fn test_update_completion() {
        let store = setup_sqlitedb().await;
        let mut to_insert = NewNote::new("test");
        to_insert.completed = true;
        store.insert_note(to_insert).await.unwrap();
        let n = Note::from_pretty(&store, " - [ ] :1: hi")
            .await
            .unwrap()
            .unwrap();
        assert!(!n.completed)
    }
    #[tokio::test]
    async fn test_invalid_id_fail() {
        let store = setup_sqlitedb().await;
        store.insert_note(NewNote::new("test")).await.unwrap();
        let n = Note::from_pretty(&store, " - [ ] :42: hi").await;
        assert!(n.is_err())
    }
}
