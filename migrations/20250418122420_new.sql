-- Add migration script here
--
CREATE table note (
    id INTEGER PRIMARY KEY NOT NULL,
    body TEXT NOT NULL,
    completed INTEGER NOT NULL,
    created_at DATETIMETZ NOT NULL DEFAULT (datetime ('now')),
    updated_at DATETIMETZ,
    deleted_at DATETIMETZ,
    day_key INTEGER NOT NULL,
    FOREIGN KEY (day_key) REFERENCES day (id)
);

CREATE table day (
    id INTEGER PRIMARY KEY NOT NULL,
    task_count INTEGER NOT NULL,
    date DATE NOT NULL UNIQUE,
    day_text TEXT NOT NULL
);

CREATE table schedule (
    id INTEGER PRIMARY KEY NOT NULL,
    hour TEXT NOT NULL,
    day TEXT NOT NULL,
    week TEXT NOT NULL
);
