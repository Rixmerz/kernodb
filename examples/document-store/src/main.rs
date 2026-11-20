//! A narrated run of the document store: insert, query, search, update, delete.

use document_store::{Doc, DocumentStore, Error};

fn doc(slug: &str, title: &str, author: &str, status: &str, year: i64, body: &str) -> Doc {
    let s = str::to_owned;
    Doc { slug: s(slug), title: s(title), author: s(author), status: s(status), year, body: s(body) }
}

fn slugs(docs: &[Doc]) -> Vec<&str> {
    docs.iter().map(|d| d.slug.as_str()).collect()
}

fn main() -> Result<(), Error> {
    let dir = std::env::temp_dir().join(format!("kerno-example-document-store-demo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let store = DocumentStore::open(&dir)?;
    store.insert(&doc(
        "backup-runbook",
        "Backup runbook",
        "ops",
        "published",
        2024,
        "Backups run every night at 02:00. A backup is kept for thirty days, and every backup is checked after it is written.",
    ))?;
    store.insert(&doc(
        "retention-policy",
        "Retention policy",
        "legal",
        "published",
        2023,
        "Personal data is kept for five years, then deleted.",
    ))?;
    store.insert(&doc("upgrade-notes", "Upgrade notes", "ops", "draft", 2025, "Take a backup before any upgrade."))?;

    store.close()?;
    let store = DocumentStore::open(&dir)?;

    let runbook = store.get("backup-runbook")?.expect("inserted above");
    println!("get: {} by {} ({})", runbook.title, runbook.author, runbook.year);
    let ops = store.by_author("ops")?;
    println!("by author ops: {:?}", slugs(&ops));
    assert_eq!(slugs(&ops), ["backup-runbook", "upgrade-notes"]);
    let recent = store.by_year(2024, 2025, 10)?;
    println!("by year 2024..=2025: {:?}", slugs(&recent));
    assert_eq!(recent.len(), 2);
    let published = store.published()?;
    println!("published: {:?}", slugs(&published));
    assert_eq!(slugs(&published), ["backup-runbook", "retention-policy"]);
    let hits = store.search("backup", 10)?;
    println!("search \"backup\": {:?}", slugs(&hits));
    let mut found = slugs(&hits);
    found.sort_unstable();
    assert_eq!(found, ["backup-runbook", "upgrade-notes"]);

    let mut notes = store.get("upgrade-notes")?.expect("inserted above");
    notes.status = "published".into();
    notes.body = "Upgrades are applied after a snapshot of the data directory.".into();
    assert!(store.update(&notes)?);
    println!("after update, search \"backup\": {:?}", slugs(&store.search("backup", 10)?));
    assert_eq!(slugs(&store.search("backup", 10)?), ["backup-runbook"]);
    assert_eq!(store.published()?.len(), 3);

    assert!(store.delete("retention-policy")?);
    assert!(store.get("retention-policy")?.is_none());
    assert!(store.by_author("legal")?.is_empty());
    assert!(store.search("personal", 10)?.is_empty());
    println!("documents left: {}", store.published()?.len());

    store.close()?;
    std::fs::remove_dir_all(&dir).expect("remove the demo database");
    Ok(())
}
