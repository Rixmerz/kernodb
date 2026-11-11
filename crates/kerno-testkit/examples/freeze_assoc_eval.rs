//! Writes the frozen evaluation set of the associative memory (E7-00) to
//! `data/assoc-eval/`: `corpus.txt`, `queries.txt` and `MANIFEST`.
//!
//! The files are committed. Running this again must change nothing; the test
//! `tests/assoc_eval.rs` checks that the committed files are what it writes.
//!
//! ```text
//! cargo run --release -p kerno-testkit --example freeze_assoc_eval
//! ```

use std::path::Path;

use kerno_testkit::assoc_eval::{SEED, Scale, Story, manifest_text, render_corpus, render_queries};

fn main() -> std::io::Result<()> {
    let story = Story::generate(Scale::Quality, SEED);
    let corpus = render_corpus(&story.corpus);
    let queries = render_queries(&story.queries);
    let manifest = manifest_text(&[("corpus.txt", corpus.as_bytes()), ("queries.txt", queries.as_bytes())]);

    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/assoc-eval");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("corpus.txt"), corpus)?;
    std::fs::write(dir.join("queries.txt"), queries)?;
    std::fs::write(dir.join("MANIFEST"), manifest)?;
    println!("wrote {}", dir.display());
    Ok(())
}
