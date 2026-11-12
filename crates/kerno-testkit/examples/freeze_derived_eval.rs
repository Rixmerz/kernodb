//! Writes the frozen evaluation set of the derived data and the vector search
//! (E8-01) to `data/derived-eval/`: `corpus.txt`, `vectors.txt`, `queries.txt`
//! and `MANIFEST`.
//!
//! The files are committed. Running this again must change nothing; the test
//! `tests/derived_eval.rs` checks that the committed files are what it writes.
//!
//! ```text
//! cargo run --release -p kerno-testkit --example freeze_derived_eval
//! ```

use std::path::Path;

use kerno_testkit::assoc_eval::manifest_text;
use kerno_testkit::derived_eval::{SEED, Scale, Story, render_corpus, render_queries, render_vectors};

fn main() -> std::io::Result<()> {
    let story = Story::generate(Scale::Quality, SEED);
    let corpus = render_corpus(&story.corpus);
    let vectors = render_vectors(&story.vectors);
    let queries = render_queries(&story.queries);
    let manifest = manifest_text(&[
        ("corpus.txt", corpus.as_bytes()),
        ("vectors.txt", vectors.as_bytes()),
        ("queries.txt", queries.as_bytes()),
    ]);

    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/derived-eval");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("corpus.txt"), corpus)?;
    std::fs::write(dir.join("vectors.txt"), vectors)?;
    std::fs::write(dir.join("queries.txt"), queries)?;
    std::fs::write(dir.join("MANIFEST"), manifest)?;
    println!("wrote {}", dir.display());
    Ok(())
}
