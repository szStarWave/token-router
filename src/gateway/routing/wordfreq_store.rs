//! SQLite-backed common-word tables for lexical rarity detection + runtime learning.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::Duration;

use anyhow::Context;
use rusqlite::{params, Connection};
use tokenizers::Tokenizer;
use wordfreq::{Standardizer, WordFreq, word_weights_from_text};

use crate::gateway::experience::RequestOutcome;
use crate::gateway::routing::step_kind::StepKind;

use super::lexical::{detect_lexical_lang, token_counts_for_rarity, LexicalLang};
use super::lexical_tokenizer::{build_wordlevel, tokenize_text};
use super::wordfreq_seed::{SEED_VERSION, SEED_WORDS};

const SCHEMA_VERSION: i32 = 2;
const LEARNED_WEIGHT: i32 = 1;
const PROMOTED_WEIGHT: i32 = 10;

#[derive(Debug, Clone)]
pub struct WordFreqSettings {
    pub enabled: bool,
    pub max_learned_per_lang: u32,
    pub min_seen_to_promote: u32,
    pub max_tokens_per_observation: u32,
}

impl Default for WordFreqSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            max_learned_per_lang: 5000,
            min_seen_to_promote: 3,
            max_tokens_per_observation: 32,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LexicalLearnContext {
    pub last_user_text: String,
    pub intent_easy: bool,
    pub rare_lexical: bool,
    pub special_lexical: bool,
}

struct LangModels {
    wordfreq: WordFreq,
    tokenizer: Tokenizer,
    words: Vec<String>,
}

pub struct WordFreqStore {
    path: PathBuf,
    conn: Mutex<Connection>,
    en: RwLock<LangModels>,
    zh: RwLock<LangModels>,
    ja: RwLock<LangModels>,
    ko: RwLock<LangModels>,
    settings: Mutex<WordFreqSettings>,
    dirty: AtomicBool,
}

impl WordFreqStore {
    pub fn open(data_dir: &Path, settings: WordFreqSettings) -> anyhow::Result<std::sync::Arc<Self>> {
        std::fs::create_dir_all(data_dir)
            .with_context(|| format!("create wordfreq dir {}", data_dir.display()))?;
        let path = data_dir.join("wordfreq.db");
        let conn = Connection::open(&path)
            .with_context(|| format!("open wordfreq db {}", path.display()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=5000;")
            .context("wordfreq db pragmas")?;
        migrate(&conn)?;
        seed_if_empty(&conn)?;
        upsert_seed_if_new_version(&conn)?;
        Ok(std::sync::Arc::new(Self::from_connection(path, conn, settings)?))
    }

    #[cfg(test)]
    pub fn open_in_memory() -> anyhow::Result<Self> {
        Self::open_in_memory_with_settings(WordFreqSettings::default())
    }

    #[cfg(test)]
    pub fn open_in_memory_with_settings(settings: WordFreqSettings) -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory().context("open in-memory wordfreq db")?;
        migrate(&conn)?;
        seed_if_empty(&conn)?;
        upsert_seed_if_new_version(&conn)?;
        Self::from_connection(PathBuf::from(":memory:"), conn, settings)
    }

    pub fn update_settings(&self, settings: WordFreqSettings) {
        *self.settings.lock().expect("wordfreq settings mutex") = settings;
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn word_frequency(&self, lang: LexicalLang, token: &str) -> f32 {
        let models = self.models_for(lang);
        let guard = models.read().expect("wordfreq models lock");
        guard.wordfreq.word_frequency(token)
    }

    pub fn tokenize(&self, text: &str, lang: LexicalLang) -> Vec<String> {
        let models = self.models_for(lang);
        let guard = models.read().expect("wordfreq models lock");
        tokenize_text(&guard.tokenizer, &guard.words, text, lang)
            .into_iter()
            .filter(|t| token_counts_for_rarity(t))
            .collect()
    }

    pub fn observe_casual(&self, text: &str) {
        let settings = self.settings();
        if !settings.enabled {
            return;
        }
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        let lang = detect_lexical_lang(trimmed);
        let tokens = self.tokenize(trimmed, lang);
        let _ = self.upsert_tokens(lang, &tokens, "learned", &settings);
    }

    pub fn reinforce_from_outcome(
        &self,
        ctx: &LexicalLearnContext,
        step_kind: StepKind,
        outcome: RequestOutcome,
    ) {
        let settings = self.settings();
        if !settings.enabled {
            return;
        }
        if ctx.special_lexical || ctx.rare_lexical {
            return;
        }
        if !outcome.edge_ok || outcome.cascade_fallback {
            return;
        }
        if !matches!(step_kind, StepKind::DirectChat | StepKind::HeartbeatAck) {
            return;
        }
        let trimmed = ctx.last_user_text.trim();
        if trimmed.is_empty() {
            return;
        }
        let lang = detect_lexical_lang(trimmed);
        let tokens = self.tokenize(trimmed, lang);
        let _ = self.upsert_tokens(lang, &tokens, "learned", &settings);
    }

    pub fn spawn_flush_task(self: &std::sync::Arc<Self>) {
        let store = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if let Err(e) = store.flush_if_dirty() {
                    tracing::warn!(error = %e, "wordfreq flush failed");
                }
            }
        });
    }

    pub fn flush_if_dirty(&self) -> anyhow::Result<()> {
        if !self.dirty.swap(false, Ordering::AcqRel) {
            return Ok(());
        }
        let conn = self.conn.lock().expect("wordfreq db mutex");
        conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);")
            .context("wordfreq wal checkpoint")?;
        Ok(())
    }

    fn settings(&self) -> WordFreqSettings {
        self.settings
            .lock()
            .expect("wordfreq settings mutex")
            .clone()
    }

    fn models_for(&self, lang: LexicalLang) -> &RwLock<LangModels> {
        match lang {
            LexicalLang::En => &self.en,
            LexicalLang::Zh => &self.zh,
            LexicalLang::Ja => &self.ja,
            LexicalLang::Ko => &self.ko,
        }
    }

    fn from_connection(
        path: PathBuf,
        conn: Connection,
        settings: WordFreqSettings,
    ) -> anyhow::Result<Self> {
        let mutex = Mutex::new(conn);
        let models = |lang| {
            let guard = mutex.lock().expect("wordfreq db mutex");
            load_lang_models(&guard, lang)
        };
        Ok(Self {
            path,
            en: RwLock::new(models(LexicalLang::En)?),
            zh: RwLock::new(models(LexicalLang::Zh)?),
            ja: RwLock::new(models(LexicalLang::Ja)?),
            ko: RwLock::new(models(LexicalLang::Ko)?),
            conn: mutex,
            settings: Mutex::new(settings),
            dirty: AtomicBool::new(false),
        })
    }

    fn upsert_tokens(
        &self,
        lang: LexicalLang,
        tokens: &[String],
        source: &str,
        settings: &WordFreqSettings,
    ) -> anyhow::Result<bool> {
        let lang_str = lang_code(lang);
        let now = now_unix();
        let mut changed = false;
        let mut unique = Vec::new();
        for token in tokens.iter().take(settings.max_tokens_per_observation as usize) {
            if !token_counts_for_rarity(token) {
                continue;
            }
            if unique.iter().any(|t: &String| t == token) {
                continue;
            }
            unique.push(token.clone());
        }
        if unique.is_empty() {
            return Ok(false);
        }

        let conn = self.conn.lock().expect("wordfreq db mutex");
        for token in unique {
            let exists: bool = conn
                .query_row(
                    "SELECT 1 FROM word_freq WHERE lang = ?1 AND word = ?2",
                    params![lang_str, token],
                    |_| Ok(()),
                )
                .is_ok();
            if !exists {
                let learned_count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM word_freq WHERE lang = ?1 AND source = 'learned'",
                    params![lang_str],
                    |row| row.get(0),
                )?;
                if learned_count >= settings.max_learned_per_lang as i64 {
                    continue;
                }
            }
            let rows = conn.execute(
                "INSERT INTO word_freq (lang, word, weight, source, seen_count, last_seen_unix)
                 VALUES (?1, ?2, ?3, ?4, 1, ?5)
                 ON CONFLICT(lang, word) DO UPDATE SET
                   seen_count = word_freq.seen_count + 1,
                   last_seen_unix = excluded.last_seen_unix,
                   weight = CASE
                     WHEN word_freq.seen_count + 1 >= ?6 AND word_freq.weight < ?7 THEN ?7
                     ELSE word_freq.weight
                   END",
                params![
                    lang_str,
                    token,
                    LEARNED_WEIGHT,
                    source,
                    now,
                    settings.min_seen_to_promote,
                    PROMOTED_WEIGHT,
                ],
            )?;
            if rows > 0 {
                changed = true;
            }
        }
        drop(conn);

        if changed {
            self.reload_lang(lang)?;
            self.dirty.store(true, Ordering::Release);
        }
        Ok(changed)
    }

    fn reload_lang(&self, lang: LexicalLang) -> anyhow::Result<()> {
        let models = {
            let conn = self.conn.lock().expect("wordfreq db mutex");
            load_lang_models(&conn, lang)?
        };
        *self.models_for(lang).write().expect("wordfreq models lock") = models;
        Ok(())
    }
}

fn lang_code(lang: LexicalLang) -> &'static str {
    match lang {
        LexicalLang::En => "en",
        LexicalLang::Zh => "zh",
        LexicalLang::Ja => "ja",
        LexicalLang::Ko => "ko",
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn migrate(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS word_freq (
            lang TEXT NOT NULL,
            word TEXT NOT NULL,
            weight INTEGER NOT NULL,
            PRIMARY KEY (lang, word)
        );
        CREATE INDEX IF NOT EXISTS idx_word_freq_lang ON word_freq(lang);",
    )
    .context("wordfreq schema")?;

    let version: i32 = meta_i32(conn, "schema_version").unwrap_or(0);
    if version < 2 {
        add_column_if_missing(conn, "word_freq", "source", "TEXT NOT NULL DEFAULT 'seed'")?;
        add_column_if_missing(conn, "word_freq", "seen_count", "INTEGER NOT NULL DEFAULT 0")?;
        add_column_if_missing(conn, "word_freq", "last_seen_unix", "INTEGER NOT NULL DEFAULT 0")?;
        set_meta(conn, "schema_version", SCHEMA_VERSION)?;
    }
    Ok(())
}

fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    def: &str,
) -> anyhow::Result<()> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info(?) WHERE name = ?",
        params![table, column],
        |row| row.get(0),
    )?;
    if count == 0 {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {def}"),
            [],
        )
        .with_context(|| format!("add column {column}"))?;
    }
    Ok(())
}

fn meta_i32(conn: &Connection, key: &str) -> anyhow::Result<i32> {
    conn.query_row(
        "SELECT value FROM schema_meta WHERE key = ?1",
        params![key],
        |row| row.get::<_, String>(0),
    )
    .map(|s| s.parse().unwrap_or(0))
    .map_err(Into::into)
}

fn set_meta(conn: &Connection, key: &str, value: i32) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO schema_meta(key, value) VALUES(?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value.to_string()],
    )?;
    Ok(())
}

fn seed_if_empty(conn: &Connection) -> anyhow::Result<()> {
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM word_freq", [], |row| row.get(0))
        .context("count word_freq rows")?;
    if count > 0 {
        return Ok(());
    }
    insert_seed_words(conn)?;
    set_meta(conn, "seed_version", SEED_VERSION)?;
    tracing::info!(count = SEED_WORDS.len(), "seeded default wordfreq tables");
    Ok(())
}

fn upsert_seed_if_new_version(conn: &Connection) -> anyhow::Result<()> {
    let stored = meta_i32(conn, "seed_version").unwrap_or(0);
    if stored >= SEED_VERSION {
        return Ok(());
    }
    insert_seed_words_ignore(conn)?;
    set_meta(conn, "seed_version", SEED_VERSION)?;
    tracing::info!(seed_version = SEED_VERSION, "merged new seed words into wordfreq db");
    Ok(())
}

fn insert_seed_words(conn: &Connection) -> anyhow::Result<()> {
    let tx = conn
        .unchecked_transaction()
        .context("wordfreq seed transaction")?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO word_freq(lang, word, weight, source, seen_count, last_seen_unix)
             VALUES (?1, ?2, ?3, 'seed', 0, 0)",
        )?;
        for entry in SEED_WORDS {
            stmt.execute(params![entry.lang, entry.word, entry.weight])?;
        }
    }
    tx.commit().context("commit wordfreq seed")?;
    Ok(())
}

fn insert_seed_words_ignore(conn: &Connection) -> anyhow::Result<()> {
    let tx = conn
        .unchecked_transaction()
        .context("wordfreq seed merge transaction")?;
    {
        let mut stmt = tx.prepare(
            "INSERT OR IGNORE INTO word_freq(lang, word, weight, source, seen_count, last_seen_unix)
             VALUES (?1, ?2, ?3, 'seed', 0, 0)",
        )?;
        for entry in SEED_WORDS {
            stmt.execute(params![entry.lang, entry.word, entry.weight])?;
        }
    }
    tx.commit().context("commit wordfreq seed merge")?;
    Ok(())
}

fn load_lang_models(conn: &Connection, lang: LexicalLang) -> anyhow::Result<LangModels> {
    let lang_str = lang_code(lang);
    let mut stmt = conn.prepare(
        "SELECT word, weight FROM word_freq WHERE lang = ?1 ORDER BY weight DESC, length(word) DESC",
    )?;
    let rows = stmt.query_map(params![lang_str], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut table = String::new();
    let mut words = Vec::new();
    for row in rows {
        let (word, weight) = row?;
        words.push(word.clone());
        table.push_str(&word);
        table.push(' ');
        table.push_str(&weight.to_string());
        table.push('\n');
    }
    Ok(LangModels {
        wordfreq: build_wordfreq(&table, lang_str),
        tokenizer: build_wordlevel(&words, lang)?,
        words,
    })
}

fn build_wordfreq(table: &str, lang: &str) -> WordFreq {
    let weights = word_weights_from_text(table.as_bytes()).expect("wordfreq table must parse");
    let standardizer = Standardizer::new(lang).expect("wordfreq standardizer");
    WordFreq::new(weights).standardizer(standardizer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observe_casual_adds_oov_and_reload() {
        let store = WordFreqStore::open_in_memory().unwrap();
        store.observe_casual("hello xyzzyplugh day");
        let lang = LexicalLang::En;
        let wf = store.word_frequency(lang, "xyzzyplugh");
        assert!(wf > 0.0);
    }

    #[test]
    fn seed_does_not_overwrite_learned_weight() {
        let store = WordFreqStore::open_in_memory().unwrap();
        store
            .upsert_tokens(
                LexicalLang::En,
                &["customword".to_string()],
                "learned",
                &WordFreqSettings::default(),
            )
            .unwrap();
        assert!(store.word_frequency(LexicalLang::En, "customword") > 0.0);
    }
}
