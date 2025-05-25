use std::str::{FromStr, Lines};

use crate::store::{NoteRow, NoteRowDate, NoteStore};
use ansi_term::{Color, Style};
use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, NaiveDate, Utc};

#[derive(Debug)]
pub enum ParsedNote {
    Note(Note),
    NewNote(NewNote),
}
impl ParsedNote {
    pub fn is_new_note(&self) -> bool {
        match self {
            ParsedNote::NewNote(_) => true,
            ParsedNote::Note(_) => false,
        }
    }
    pub fn new_note(self) -> Option<NewNote> {
        match self {
            ParsedNote::NewNote(n) => Some(n),
            ParsedNote::Note(_) => None,
        }
    }
    pub fn note(self) -> Option<Note> {
        match self {
            ParsedNote::NewNote(_) => None,
            ParsedNote::Note(n) => Some(n),
        }
    }
    pub fn is_note(&self) -> bool {
        !self.is_new_note()
    }
    pub fn parse_pretty_md(s: impl AsRef<str>) -> Result<Option<ParsedNote>> {
        let s = s.as_ref();
        let s = s.trim();
        if s.len() < 7 {
            return Err(anyhow!("Invalid note start, not long enough. {}", &s));
        }
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
                if body.is_empty() {
                    return Ok(None);
                }
                let id = id_string.parse::<u32>().context(format!(
                    "Parsing {} failed. {}",
                    id_string,
                    &s[idx + 1..]
                ))?;
                return Ok(Some(ParsedNote::Note(Note {
                    id,
                    body,
                    completed,
                })));
            }
            None => {
                let new_note_text = s[idx + 1..].trim();
                if new_note_text.is_empty() {
                    return Ok(None);
                }
                return Ok(Some(ParsedNote::NewNote(NewNote {
                    body: String::from(new_note_text),
                    completed,
                    created_at: Utc::now(),
                })));
            }
        }
    }
}

#[derive(Debug)]
pub struct Note {
    pub id: u32,
    pub body: String,
    pub completed: bool,
}
impl From<NoteRow> for Note {
    fn from(value: NoteRow) -> Self {
        Note {
            id: value.id,
            body: value.body,
            completed: value.completed,
        }
    }
}
impl From<NoteRowDate> for Note {
    fn from(value: NoteRowDate) -> Self {
        Note {
            id: value.id,
            body: value.body,
            completed: value.completed,
        }
    }
}
impl Note {
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
#[derive(Debug)]
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
        out.push_str("---");
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
pub struct ParsedDayNotes {
    pub notes: Vec<ParsedNote>,
    pub note_count: u32,
    pub date: NaiveDate,
    pub day_text: String,
}
impl ParsedDayNotes {
    pub fn parse_pretty_md(line_iter: &mut Lines<'_>) -> Result<ParsedDayNotes> {
        let mut date: Option<&str> = None;
        // Iterate through lines till find the date prefix!
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
        let date = NaiveDate::from_str(date)?;
        let mut day_text = String::new();
        let mut notes = vec![];
        // Update notes by line.
        for line in line_iter {
            // exit the iteration if end of day note is found.
            if line == "---" {
                break;
            }
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match line.chars().next().unwrap() {
                '-' => {
                    let Ok(Some(n)) = ParsedNote::parse_pretty_md(line) else {
                        continue;
                    };
                    notes.push(n);
                }
                _ => {
                    day_text.push_str(line);
                    day_text.push_str("\n");
                }
            }
        }
        let note_count = notes.len() as u32;
        Ok(ParsedDayNotes {
            notes,
            note_count,
            date,
            day_text,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{fs::File, io::Read, str::FromStr};

    use crate::{
        notes::{NewNote, Note},
        store::setup_db,
    };
    use chrono::NaiveDate;
    use sqlx::migrate;

    use super::{ParsedDayNotes, ParsedNote};

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
    #[test]
    fn test_parse_none() {
        let table = vec![" - [ ] :", " - [x] :1:", " - [x] :"];
        for input in table {
            println!("{}", input);
            let note = ParsedNote::parse_pretty_md(input).unwrap();
            assert!(note.is_none());
        }
    }
    #[test]
    fn test_parse_new_notes() {
        let table = vec![
            ((false, "hi"), " - [ ] : hi"),
            ((true, "hi"), " - [x] :hi "),
            ((true, "1 text with spaces"), " - [x] :1 text with spaces"),
        ];
        for ((comp, text), input) in table {
            println!("{}", input);
            let note = ParsedNote::parse_pretty_md(input)
                .unwrap()
                .unwrap()
                .new_note()
                .unwrap();
            assert_eq!(note.completed, comp, "{}", input);
            assert_eq!(note.body, text, "{}", input);
        }
    }
    #[test]
    fn test_parse_notes() {
        let table = vec![
            ((false, 42, "hi"), " - [ ] :42: hi"),
            ((true, 34, "hi"), " - [x] :34: hi"),
            (
                (true, 123456908, "text with spaces"),
                " - [x] :123456908: text with spaces",
            ),
        ];
        for ((comp, id, text), input) in table {
            let note = ParsedNote::parse_pretty_md(input)
                .unwrap()
                .unwrap()
                .note()
                .unwrap();
            assert_eq!(note.completed, comp);
            assert_eq!(note.id, id);
            assert_eq!(note.body, text);
        }
    }
    #[test]
    fn test_parse_notes_fail() {
        let table = vec![
            "-[] :  ",
            "[]",
            " - [ ];34;",
            "dj;salkj",
            ";saldjf;asljdf;as",
            " - [ ];34: test",
            " - [ ]:34; test",
            " - [ ]:hi: test",
        ];
        for input in table {
            let note = ParsedNote::parse_pretty_md(input);
            assert!(note.is_err(), "{}", input);
        }
    }
    #[test]
    fn test_parse_day_note() {
        let mut input = String::new();
        File::open("test/day_notes.md")
            .unwrap()
            .read_to_string(&mut input)
            .unwrap();
        println!("{}", input);
        let mut lines = input.lines();
        let notes = ParsedDayNotes::parse_pretty_md(&mut lines).unwrap();
        assert_eq!(notes.notes.len(), 0);
        assert_eq!(notes.date, NaiveDate::from_str("12-10-25").unwrap());
    }
    #[test]
    fn test_parse_day_note_double() {
        let mut input = String::new();
        File::open("test/day_notes.md")
            .unwrap()
            .read_to_string(&mut input)
            .unwrap();
        println!("{}", input);
        let mut lines = input.lines();
        ParsedDayNotes::parse_pretty_md(&mut lines).unwrap();
        let notes = ParsedDayNotes::parse_pretty_md(&mut lines).unwrap();
        assert_eq!(notes.notes.len(), 2);
        assert_eq!(notes.date, NaiveDate::from_str("12-10-25").unwrap());
        assert!(notes.notes[0].is_note(), "{:?}", notes.notes);
        assert!(notes.notes[1].is_new_note());
    }
}
